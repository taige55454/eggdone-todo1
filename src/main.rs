#![allow(warnings)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod models;
use models::*;

use chrono::{Datelike, Duration, Local, NaiveDate, Timelike};
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, Pos2, Rect, RichText, Rounding, Sense, Stroke, Vec2};
use notify_rust::Notification;
use single_instance::SingleInstance;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::time::Instant;
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon as TrayIconData, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

#[derive(PartialEq, Clone, Copy)]
enum ViewTab {
    All,
    Quadrants,
    Plans,
    Forest,
}

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

struct EggDoneApp {
    config: Config,
    todos: Vec<TodoTask>,
    history: Vec<HistoryRecord>,
    plans: Vec<Plan>,
    cached_forest_records: Vec<ForestRecord>,

    is_expanded: bool,
    is_rest_popup_mode: bool,
    current_tab: ViewTab,
    input_text: String,
    parsed_preview: String,

    pending_confirm: HashMap<String, Instant>,
    toast: Option<(String, Instant)>,

    // 专注时钟相关
    focus_minutes: u32,
    remaining_secs: u32,
    is_focus_running: bool,
    is_resting: bool,
    current_focus_task: Option<(String, String)>,
    last_sec_tick: Instant,
    ball_icon: String,
    ball_time_str: String,

    // UI时钟缓存：彻底消灭渲染循环的内存碎片
    cached_today_str: String,
    cached_now_hm: String,
    last_clock_update: Instant,

    edit_modal: Option<EditDialogState>,
    show_history_modal: bool,
    show_forest_stats_modal: bool,
    show_popup_settings_modal: bool,
    confirm_delete_task: Option<(String, String)>,
    confirm_delete_plan: Option<(String, String)>,
    confirm_giveup_focus: bool,

    last_sync_date: NaiveDate,
    last_reminder_check: Instant,
    mouse_leave_time: Option<Instant>,
    hover_enter_time: Option<Instant>,
    ball_pos: Pos2,
    panel_pos: Pos2,
    has_dragged: bool,
    autostart_enabled: bool,
    allow_quit: bool,

    _tray_icon: Option<TrayIcon>,
    tray_show_id: Option<tray_icon::menu::MenuId>,
    tray_auto_item: Option<CheckMenuItem>,
    tray_quit_id: Option<tray_icon::menu::MenuId>,
}

fn create_tray_icon_data() -> Option<TrayIconData> {
    let size = 32u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let center = 15.5f32;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= 13.5 {
                if dist >= 11.2 {
                    rgba.extend_from_slice(&[255, 213, 79, 255]);
                } else {
                    rgba.extend_from_slice(&[46, 125, 50, 255]);
                }
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    TrayIconData::from_rgba(rgba, size, size).ok()
}

impl EggDoneApp {
    fn new(cc: &eframe::CreationContext<'_>, init_ball_pos: Pos2) -> Self {
        setup_custom_fonts(&cc.egui_ctx);

        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::light();
        style.visuals.window_rounding = Rounding::same(12.0);
        style.visuals.window_fill = Color32::from_rgb(250, 252, 250);
        style.visuals.panel_fill = Color32::TRANSPARENT;
        style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(225, 232, 225));
        style.visuals.widgets.inactive.rounding = Rounding::same(6.0);
        style.visuals.widgets.hovered.rounding = Rounding::same(6.0);
        style.visuals.widgets.active.rounding = Rounding::same(6.0);
        cc.egui_ctx.set_style(style);

        let config: Config = safe_json_load("config.json");
        let todos: Vec<TodoTask> = safe_json_load("todos.json");
        let history: Vec<HistoryRecord> = safe_json_load("history.json");
        let plans: Vec<Plan> = safe_json_load("plans.json");
        let focus_mins = config.focus_minutes.clamp(1, 120);
        let autostart = is_autostart_enabled();

        let mut tray_icon_holder = None;
        let mut show_id = None;
        let mut auto_item_holder = None;
        let mut quit_id = None;

        let tray_menu = Menu::new();
        let item_show = MenuItem::new("🖥️ 显示主面板", true, None);
        let item_auto = CheckMenuItem::new("⚡ 开机自动启动", true, autostart, None);
        let item_quit = MenuItem::new("✖ 彻底退出", true, None);
        show_id = Some(item_show.id().clone());
        quit_id = Some(item_quit.id().clone());
        let _ = tray_menu.append(&item_show);
        let _ = tray_menu.append(&item_auto);
        let _ = tray_menu.append(&PredefinedMenuItem::separator());
        let _ = tray_menu.append(&item_quit);
        auto_item_holder = Some(item_auto);

        if let Some(icon_data) = create_tray_icon_data() {
            if let Ok(tray) = TrayIconBuilder::new()
                .with_menu(Box::new(tray_menu))
                .with_tooltip("EggDone 待办与专注森林 (左键点击展开)")
                .with_icon(icon_data)
                .build()
            {
                tray_icon_holder = Some(tray);
            }
        }

        let local_now = Local::now();

        let mut app = Self {
            config,
            todos,
            history,
            plans,
            cached_forest_records: Vec::new(),
            is_expanded: false,
            is_rest_popup_mode: false,
            current_tab: ViewTab::All,
            input_text: String::new(),
            parsed_preview: String::new(),
            pending_confirm: HashMap::new(),
            toast: None,
            focus_minutes: focus_mins,
            remaining_secs: focus_mins * 60,
            is_focus_running: false,
            is_resting: false,
            current_focus_task: None,
            last_sec_tick: Instant::now(),
            ball_icon: "🥚".to_string(),
            ball_time_str: String::new(),
          
            cached_today_str: local_now.format("%Y-%m-%d").to_string(),
            cached_now_hm: local_now.format("%H:%M").to_string(),
            last_clock_update: Instant::now(),

            edit_modal: None,
            show_history_modal: false,
            show_forest_stats_modal: false,
            show_popup_settings_modal: false,
            confirm_delete_task: None,
            confirm_delete_plan: None,
            confirm_giveup_focus: false,
            last_sync_date: local_now.date_naive(),
            last_reminder_check: Instant::now(),
            mouse_leave_time: None,
            hover_enter_time: None,
            ball_pos: init_ball_pos,
            panel_pos: Pos2::new((init_ball_pos.x - 300.0).max(20.0), init_ball_pos.y),
            has_dragged: false,
            autostart_enabled: autostart,
            allow_quit: false,
            _tray_icon: tray_icon_holder,
            tray_show_id: show_id,
            tray_auto_item: auto_item_holder,
            tray_quit_id: quit_id,
        };

        app.migrate_and_sync_data();
        app
    }

    fn migrate_and_sync_data(&mut self) {
        let mut changed = false;
        let today = today_str();
        for t in &mut self.todos {
            if t.uuid.is_empty() { t.uuid = gen_uuid(); changed = true; }
            if t.cycle_date.is_empty() {
                t.cycle_date = t.due_date.clone().unwrap_or_else(|| today.clone());
                changed = true;
            }
            if let Some(ref rule) = t.repeat_rule {
                if t.plan_uuid.is_none() {
                    let p_uuid = gen_uuid();
                    t.plan_uuid = Some(p_uuid.clone());
                    self.plans.push(Plan {
                        plan_uuid: p_uuid,
                        text: t.text.clone(),
                        priority: t.priority,
                        due_time: t.due_time.clone(),
                        reminder_time: t.reminder_time.clone(),
                        repeat_rule: rule.clone(),
                        repeat_days: t.repeat_days.clone().unwrap_or_default(),
                        base_date: t.due_date.clone().unwrap_or_else(|| today.clone()),
                    });
                    changed = true;
                }
            }
        }
        if self.sync_repeat_plans() || changed {
            self.save_all_data();
        }
        self.sort_todos();
    }

    fn save_all_data(&self) {
        safe_json_dump("config.json", &self.config);
        safe_json_dump("todos.json", &self.todos);
        safe_json_dump("history.json", &self.history);
        safe_json_dump("plans.json", &self.plans);
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
        let today_s = today.format("%Y-%m-%d").to_string();

        let mut existing: HashSet<(String, String)> = HashSet::new();
        for t in &self.todos {
            if let Some(ref pu) = t.plan_uuid {
                existing.insert((pu.clone(), t.cycle_date.clone()));
            }
        }
        for h in self.history.iter().rev().take(300) {
            if let (Some(ref pu), Some(ref cd)) = (&h.plan_uuid, &h.cycle_date) {
                existing.insert((pu.clone(), cd.clone()));
            }
        }

        let mut added = false;
        for plan in &self.plans {
            let base_d = NaiveDate::parse_from_str(&plan.base_date, "%Y-%m-%d").unwrap_or(today);
            if today < base_d { continue; }

            let wk = today.weekday().num_days_from_monday();
            let hits = match plan.repeat_rule.as_str() {
                "daily" => true,
                "weekdays" => wk < 5,
                "weekly" => {
                    if plan.repeat_days.is_empty() {
                        wk == base_d.weekday().num_days_from_monday()
                    } else {
                        plan.repeat_days.contains(&wk)
                    }
                }
                "monthly" => {
                    let max_d = days_in_month(today.year(), today.month());
                    if plan.repeat_days.is_empty() {
                        today.day() == base_d.day().min(max_d)
                    } else {
                        plan.repeat_days.iter().any(|&d| d.min(max_d) == today.day())
                    }
                }
                _ => false,
            };

            if hits && !existing.contains(&(plan.plan_uuid.clone(), today_s.clone())) {
                self.todos.push(TodoTask {
                    uuid: gen_uuid(),
                    plan_uuid: Some(plan.plan_uuid.clone()),
                    cycle_date: today_s.clone(),
                    text: plan.text.clone(),
                    priority: plan.priority,
                    due_date: Some(today_s.clone()),
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
                added = true;
            }
        }
        if added { self.sort_todos(); }
        added
    }

    fn show_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn expand_window(&mut self, ctx: &egui::Context) {
        if self.is_expanded && !self.is_rest_popup_mode { return; }
        self.is_expanded = true;
        self.is_rest_popup_mode = false;
        self.mouse_leave_time = None;
        self.hover_enter_time = None;

        if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
            self.ball_pos = rect.min;
        }

        let monitor = ctx.input(|i| i.viewport().monitor_size).unwrap_or(Vec2::new(1920.0, 1080.0));
        let max_y = (monitor.y - 650.0).max(10.0);

        let target_x = if self.ball_pos.x > 340.0 {
            (self.ball_pos.x + 64.0 - 360.0).max(10.0)
        } else {
            self.ball_pos.x.max(10.0)
        };
        let target_y = self.ball_pos.y.clamp(10.0, max_y);
        self.panel_pos = Pos2::new(target_x, target_y);

        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(360.0, 640.0)));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(self.panel_pos));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn collapse_window(&mut self, ctx: &egui::Context) {
        if !self.is_expanded && !self.is_rest_popup_mode { return; }
        self.is_expanded = false;
        self.is_rest_popup_mode = false;
        self.mouse_leave_time = None;
        self.hover_enter_time = None;

        if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
            self.panel_pos = rect.min;
            self.ball_pos.x = if self.panel_pos.x > 10.0 { self.panel_pos.x + 360.0 - 64.0 } else { self.panel_pos.x };
            self.ball_pos.y = self.panel_pos.y;
            self.config.ball_x = self.ball_pos.x;
            self.config.ball_y = self.ball_pos.y;
            safe_json_dump("config.json", &self.config);
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(64.0, 64.0)));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(self.ball_pos));
    }

    fn trigger_rest_popup_window(&mut self, ctx: &egui::Context) {
        self.is_rest_popup_mode = true;
        self.is_expanded = false;
        let w = self.config.popup_width.clamp(240.0, 800.0);
        let h = self.config.popup_height.clamp(220.0, 600.0);

        let monitor = ctx.input(|i| i.viewport().monitor_size).unwrap_or(Vec2::new(1920.0, 1080.0));
        let pos = match self.config.popup_position.as_str() {
            "top_right" => Pos2::new((monitor.x - w - 30.0).max(20.0), 50.0),
            "bottom_right" => Pos2::new((monitor.x - w - 30.0).max(20.0), (monitor.y - h - 70.0).max(20.0)),
            "top_left" => Pos2::new(30.0, 50.0),
            "bottom_left" => Pos2::new(30.0, (monitor.y - h - 70.0).max(20.0)),
            "custom" => Pos2::new(self.config.popup_custom_x.max(0.0), self.config.popup_custom_y.max(0.0)),
            _ => Pos2::new(((monitor.x - w) * 0.5).max(20.0), ((monitor.y - h) * 0.45).max(20.0)),
        };

        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(w, h)));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn has_any_modal(&self) -> bool {
        self.edit_modal.is_some()
            || self.show_history_modal
            || self.show_forest_stats_modal
            || self.show_popup_settings_modal
            || self.confirm_delete_task.is_some()
            || self.confirm_delete_plan.is_some()
            || self.confirm_giveup_focus
    }

    fn poll_tray_and_close_events(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.collapse_window(ctx);
            self.show_toast("已最小化至桌面悬浮球与系统托盘");
        }

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if Some(&event.id) == self.tray_show_id.as_ref() {
                self.expand_window(ctx);
            } else if Some(&event.id) == self.tray_quit_id.as_ref() {
                self.allow_quit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if let Some(ref item) = self.tray_auto_item {
                if &event.id == item.id() {
                    let target = !self.autostart_enabled;
                    if set_autostart(target) {
                        self.autostart_enabled = target;
                        item.set_checked(target);
                        self.show_toast(if target { "✅ 已开启开机自启" } else { "🚫 已关闭开机自启" });
                    } else {
                        item.set_checked(self.autostart_enabled);
                        self.show_toast("❌ 权限不足，设置失败");
                    }
                }
            }
        }

        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click { button: tray_icon::MouseButton::Left, .. } = event {
                self.expand_window(ctx);
            }
        }
    }

    fn commit_new_task(&mut self) {
        let raw = self.input_text.trim().to_string();
        if raw.is_empty() { return; }

        if let Some(parsed) = QuickAddParser::parse(&raw) {
            let today_s = today_str();
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
                    base_date: parsed.base_date.clone().or_else(|| parsed.due_date.clone()).unwrap_or_else(|| today_s.clone()),
                });
            }

            let mut hide_until = None;
            if plan_uuid.is_some() {
                if let Some(ref dd) = parsed.due_date {
                    let today = Local::now().date_naive();
                    if let Ok(d_obj) = NaiveDate::parse_from_str(dd, "%Y-%m-%d") {
                        if d_obj > today {
                            let h_obj = d_obj - Duration::days(1);
                            hide_until = Some(if h_obj > today { h_obj.format("%Y-%m-%d").to_string() } else { today_s.clone() });
                        }
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
            self.show_toast("✨ 已智能添加新事项");
        }
    }

    fn complete_task(&mut self, uuid: &str) {
        if let Some(idx) = self.todos.iter().position(|t| t.uuid == uuid) {
            let task = self.todos.remove(idx);
            self.pending_confirm.remove(uuid);

            if !self.is_focus_running {
                if let Some((ref focus_uuid, _)) = self.current_focus_task {
                    if focus_uuid == uuid {
                        self.current_focus_task = None;
                    }
                }
            }

            self.history.push(HistoryRecord {
                plan_uuid: task.plan_uuid,
                cycle_date: Some(task.cycle_date),
                text: task.text,
                due_date: task.due_date,
                created: Some(task.created),
                completed: now_iso(),
            });
            self.save_all_data();
            self.show_toast("🎉 事项已达成！");
        }
    }

    fn postpone_task(&mut self, uuid: &str) {
        let today = Local::now().date_naive();
        let tomorrow_str = (today + Duration::days(1)).format("%Y-%m-%d").to_string();
        if let Some(t) = self.todos.iter_mut().find(|t| t.uuid == uuid) {
            if t.due_date.is_some() && t.original_due.is_none() {
                t.original_due = t.due_date.clone();
            }
            if let Some(ref dd) = t.due_date {
                let d_obj = NaiveDate::parse_from_str(dd, "%Y-%m-%d").unwrap_or(today);
                let new_due = (d_obj + Duration::days(1)).max(today + Duration::days(1));
                t.due_date = Some(new_due.format("%Y-%m-%d").to_string());
                t.hide_until = Some(tomorrow_str);
            } else {
                t.due_date = Some(tomorrow_str.clone());
                t.hide_until = Some(tomorrow_str);
            }
        }
        self.sort_todos();
        self.save_all_data();
        self.show_toast("➡️ 已延后一天并智能隐藏");
    }

    fn record_forest(&mut self, status: &str) {
        let mut records: Vec<ForestRecord> = safe_json_load("forest.json");
        records.push(ForestRecord {
            timestamp: now_iso(),
            status: status.to_string(),
            task: self.current_focus_task.as_ref().map(|(_, t)| t.clone()).unwrap_or_else(|| "自由专注".to_string()),
            minutes: self.focus_minutes,
        });
        safe_json_dump("forest.json", &records);
        self.cached_forest_records = records;
    }

    fn reset_forest_session(&mut self) {
        self.is_focus_running = false;
        self.is_resting = false;
        self.remaining_secs = self.focus_minutes * 60;
        self.ball_icon = "🥚".to_string();
        self.ball_time_str.clear();
    }

    fn start_rest_session(&mut self) {
        self.is_resting = true;
        self.remaining_secs = 5 * 60;
        self.is_focus_running = true;
        self.last_sec_tick = Instant::now();
        self.ball_icon = "☕".to_string();
        self.ball_time_str = "05:00".to_string();
    }

    fn tick_timers(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
      
        // 【优化】每10秒更新一次时钟缓存，彻底消除UI渲染循环中的字符串分配(GC垃圾)
        if now.duration_since(self.last_clock_update).as_secs() >= 10 {
            self.last_clock_update = now;
            let local_now = Local::now();
            self.cached_today_str = local_now.format("%Y-%m-%d").to_string();
            self.cached_now_hm = local_now.format("%H:%M").to_string();
        }

        self.pending_confirm.retain(|_, t| now.duration_since(*t).as_millis() < 2500);
        if let Some((_, t)) = &self.toast {
            if now.duration_since(*t).as_millis() > 2200 {
                self.toast = None;
            }
        }

        if self.is_focus_running {
            // 【优化】真实物理时间差补偿：即使系统强制休眠，唤醒后也能精准扣除流逝的时间
            let elapsed_secs = now.duration_since(self.last_sec_tick).as_secs();
            if elapsed_secs >= 1 {
                self.last_sec_tick += std::time::Duration::from_secs(elapsed_secs);
                self.remaining_secs = self.remaining_secs.saturating_sub(elapsed_secs as u32);
              
                let m = self.remaining_secs / 60;
                let s = self.remaining_secs % 60;
                self.ball_time_str = format!("{:02}:{:02}", m, s);

                if !self.is_resting {
                    let total = (self.focus_minutes * 60).max(1) as f32;
                    let elapsed = (self.focus_minutes * 60 - self.remaining_secs) as f32;
                    let ratio = elapsed / total;
                    self.ball_icon = if ratio < 0.25 { "🌰" } else if ratio < 0.6 { "🌱" } else if ratio < 0.9 { "🌿" } else { "🌲" }.to_string();

                    if self.remaining_secs == 0 {
                        self.is_focus_running = false;
                        self.record_forest("healthy");
                        if let Some((uuid, title)) = self.current_focus_task.clone() {
                            self.complete_task(&uuid);
                            let _ = Notification::new().summary("🌲 专注达成！").body(&format!("事项已完成：{}", title)).show();
                        } else {
                            let _ = Notification::new().summary("🌲 专注完成！").body("休息一下吧 ☕").show();
                        }
                        self.trigger_rest_popup_window(ctx);
                    }
                } else {
                    self.ball_icon = "☕".to_string();
                    if self.remaining_secs == 0 {
                        self.reset_forest_session();
                        let _ = Notification::new().summary("☕ 休息结束").body("元气满满，准备开启下一轮吧！").show();
                    }
                }
            }
        }

        if now.duration_since(self.last_reminder_check).as_secs() >= 25 {
            self.last_reminder_check = now;
            let cur_date = Local::now().date_naive();
            if cur_date != self.last_sync_date {
                self.sync_repeat_plans();
                self.save_all_data();
            }
            let d_str = self.cached_today_str.clone();
            let t_str = self.cached_now_hm.clone();
            let mut notified_any = false;
            for task in &mut self.todos {
                let r_time = task.reminder_time.as_ref().or(task.due_time.as_ref());
                if task.due_date.as_deref() == Some(d_str.as_str()) && r_time.map(|x| x.as_str()) == Some(t_str.as_str()) && !task.notified {
                    task.notified = true;
                    notified_any = true;
                    let _ = Notification::new().summary("⏰ 待办提醒").body(&task.text).show();
                }
            }
            if notified_any { self.save_all_data(); }
        }
    }

    fn copy_handover(&mut self, ctx: &egui::Context) {
        let now = Local::now();
        let tomorrow_str = (now.date_naive() + Duration::days(1)).format("%Y-%m-%d").to_string();
        let mut lines = vec![
            "【前台未完成待办交接】".to_string(),
            format!("交接时间：{}", now.format("%Y-%m-%d %H:%M")),
            "------------------------".to_string(),
        ];

        let tasks: Vec<&TodoTask> = self.todos.iter().filter(|t| {
            t.due_date.as_ref().map(|d| d.as_str() <= tomorrow_str.as_str()).unwrap_or(true)
        }).collect();

        if tasks.is_empty() {
            lines.push("当前无紧急未完成事项，辛苦了！🎉".to_string());
        } else {
            for (idx, t) in tasks.iter().enumerate() {
                let mut tm_parts = Vec::new();
                if let Some(ref d) = t.due_date { if d.chars().count() >= 5 { tm_parts.push(d.chars().skip(5).collect()); } }
                if let Some(ref tm) = t.due_time { tm_parts.push(tm.clone()); }
                let tm_str = if tm_parts.is_empty() { "不限期".to_string() } else { tm_parts.join(" ") };

                if t.repeat_rule.is_some() {
                    let r_name = get_rule_display_str(t.repeat_rule.as_deref(), t.repeat_days.as_ref());
                    lines.push(format!("{}. {} [⚠️{}任务：应于 {} 前完成，暂未处理]", idx + 1, t.text, r_name, tm_str));
                } else {
                    lines.push(format!("{}. {} （截止：{}）", idx + 1, t.text, tm_str));
                }
            }
        }
        ctx.output_mut(|o| o.copied_text = lines.join("\n"));
        self.show_toast("✅ 交接事项已成功复制！");
    }

    fn open_task_editor(&mut self, task: &TodoTask) {
        let today = Local::now().date_naive();
        let (has_due, y, m, d) = if let Some(ref dd) = task.due_date {
            let parsed = NaiveDate::parse_from_str(dd, "%Y-%m-%d").unwrap_or(today);
            (true, parsed.year(), parsed.month(), parsed.day())
        } else {
            (false, today.year(), today.month(), today.day())
        };
        let (has_time, hh, mm) = if let Some(ref tm) = task.due_time {
            let parts: Vec<&str> = tm.split(':').collect();
            let h = parts.first().and_then(|s| s.parse().ok()).unwrap_or(18);
            let m = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            (true, h, m)
        } else {
            (false, 18, 0)
        };
        let mut wk = [false; 7];
        if let Some(ref days) = task.repeat_days {
            for &day in days { if (day as usize) < 7 { wk[day as usize] = true; } }
        } else {
            wk[0] = true;
        }
        let m_str = task.repeat_days.as_ref().map(|v| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ")).unwrap_or_else(|| "1".to_string());

        self.edit_modal = Some(EditDialogState {
            is_plan: false,
            is_new_plan: false,
            is_periodic_instance: task.plan_uuid.is_some(),
            target_uuid: task.uuid.clone(),
            title: task.text.clone(),
            priority: task.priority == 1,
            has_due,
            due_year: y,
            due_month: m,
            due_day: d,
            has_time,
            time_hour: hh,
            time_min: mm,
            repeat_rule: task.repeat_rule.clone().unwrap_or_else(|| "none".to_string()),
            weekly_checked: wk,
            monthly_str: m_str,
        });
    }

    fn open_plan_editor(&mut self, plan: Option<&Plan>) {
        let today = Local::now().date_naive();
        if let Some(p) = plan {
            let (has_time, hh, mm) = if let Some(ref tm) = p.due_time {
                let parts: Vec<&str> = tm.split(':').collect();
                (true, parts.first().and_then(|s| s.parse().ok()).unwrap_or(18), parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0))
            } else {
                (false, 18, 0)
            };
            let mut wk = [false; 7];
            for &d in &p.repeat_days { if (d as usize) < 7 { wk[d as usize] = true; } }
            if !wk.iter().any(|&x| x) { wk[0] = true; }
            let m_str = if p.repeat_days.is_empty() { "1".to_string() } else { p.repeat_days.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ") };

            self.edit_modal = Some(EditDialogState {
                is_plan: true,
                is_new_plan: false,
                is_periodic_instance: false,
                target_uuid: p.plan_uuid.clone(),
                title: p.text.clone(),
                priority: p.priority == 1,
                has_due: false,
                due_year: today.year(),
                due_month: today.month(),
                due_day: today.day(),
                has_time,
                time_hour: hh,
                time_min: mm,
                repeat_rule: p.repeat_rule.clone(),
                weekly_checked: wk,
                monthly_str: m_str,
            });
        } else {
            let mut wk = [false; 7];
            wk[0] = true;
            self.edit_modal = Some(EditDialogState {
                is_plan: true,
                is_new_plan: true,
                is_periodic_instance: false,
                target_uuid: gen_uuid(),
                title: String::new(),
                priority: false,
                has_due: false,
                due_year: today.year(),
                due_month: today.month(),
                due_day: today.day(),
                has_time: false,
                time_hour: 18,
                time_min: 0,
                repeat_rule: "daily".to_string(),
                weekly_checked: wk,
                monthly_str: "1, 15".to_string(),
            });
        }
    }

    fn save_editor_changes(&mut self, st: EditDialogState) {
        let title = st.title.trim().to_string();
        if title.is_empty() {
            self.show_toast("⚠️ 名称不能为空！");
            return;
        }
        let prio = if st.priority { 1 } else { 0 };
        let due_time = if st.has_time { Some(format!("{:02}:{:02}", st.time_hour.min(23), st.time_min.min(59))) } else { None };
        let rule_opt = if st.repeat_rule == "none" { None } else { Some(st.repeat_rule.clone()) };
        let mut r_days = Vec::new();
        if st.repeat_rule == "weekly" {
            for (i, &chk) in st.weekly_checked.iter().enumerate() { if chk { r_days.push(i as u32); } }
            if r_days.is_empty() { r_days.push(0); }
        } else if st.repeat_rule == "monthly" {
            for part in st.monthly_str.replace('，', ",").split(',') {
                if let Ok(v) = part.trim().parse::<u32>() {
                    if (1..=31).contains(&v) { r_days.push(v); }
                }
            }
            r_days.sort_unstable();
            r_days.dedup();
            if r_days.is_empty() { r_days.push(1); }
        }

        if st.is_plan {
            let rule_str = rule_opt.unwrap_or_else(|| "daily".to_string());
            if st.is_new_plan {
                self.plans.push(Plan {
                    plan_uuid: st.target_uuid,
                    text: title,
                    priority: prio,
                    due_time: due_time.clone(),
                    reminder_time: due_time,
                    repeat_rule: rule_str,
                    repeat_days: r_days,
                    base_date: today_str(),
                });
                self.sync_repeat_plans();
            } else {
                for p in &mut self.plans {
                    if p.plan_uuid == st.target_uuid {
                        p.text = title.clone();
                        p.priority = prio;
                        p.due_time = due_time.clone();
                        p.repeat_rule = rule_str.clone();
                        p.repeat_days = r_days.clone();
                        break;
                    }
                }
                for t in &mut self.todos {
                    if t.plan_uuid.as_deref() == Some(st.target_uuid.as_str()) {
                        t.text = title.clone();
                        t.priority = prio;
                        t.due_time = due_time.clone();
                    }
                }
            }
        } else if let Some(t) = self.todos.iter_mut().find(|x| x.uuid == st.target_uuid) {
            t.text = title;
            t.priority = prio;
            t.due_time = due_time;
            if st.has_due {
                let max_d = days_in_month(st.due_year, st.due_month.clamp(1, 12));
                let valid_d = st.due_day.clamp(1, max_d);
                t.due_date = Some(format!("{:04}-{:02}-{:02}", st.due_year, st.due_month.clamp(1, 12), valid_d));
            } else {
                t.due_date = None;
            }
            if !st.is_periodic_instance {
                t.repeat_rule = rule_opt;
                t.repeat_days = if r_days.is_empty() { None } else { Some(r_days) };
            }
        }
        self.sort_todos();
        self.save_all_data();
        self.edit_modal = None;
        self.show_toast("✅ 保存成功");
    }

    fn render_task_card(&mut self, ui: &mut egui::Ui, task: &TodoTask, rule_display: &str, today_s: &str, now_hm: &str) {
        let is_pending = self.pending_confirm.contains_key(&task.uuid);
        let mut is_overdue = false;
        if let Some(ref dd) = task.due_date {
            if dd.as_str() < today_s {
                is_overdue = true;
            } else if dd.as_str() == today_s {
                if let Some(ref tm) = task.due_time {
                    if tm.as_str() < now_hm { is_overdue = true; }
                }
            }
        }

        let card_bg = if is_pending {
            Color32::from_rgb(227, 242, 253)
        } else if is_overdue {
            Color32::from_rgb(255, 250, 250)
        } else {
            Color32::WHITE
        };
        let border_col = if is_pending {
            Color32::from_rgb(100, 181, 246)
        } else if is_overdue {
            Color32::from_rgb(255, 205, 210)
        } else if task.priority == 1 {
            Color32::from_rgb(255, 224, 130)
        } else {
            Color32::from_rgb(232, 236, 232)
        };

        let frame = egui::Frame::none()
            .fill(card_bg)
            .rounding(Rounding::same(10.0))
            .stroke(Stroke::new(1.0, border_col))
            .inner_margin(egui::Margin::symmetric(10.0, 8.0));

        let resp = frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let icon_txt = if is_pending { "✔️" } else { "◯" };
                let icon_col = if is_pending {
                    Color32::from_rgb(21, 101, 192)
                } else if task.priority == 1 {
                    Color32::from_rgb(245, 124, 0)
                } else {
                    Color32::from_rgb(158, 158, 158)
                };
                ui.label(RichText::new(icon_txt).size(16.0).color(icon_col).strong());
                ui.add_space(2.0);

                let title_str = if is_pending { format!("【再点一次完成】{}", task.text) } else { task.text.clone() };
                let title_col = if is_pending { Color32::from_rgb(21, 101, 192) } else { Color32::from_rgb(38, 50, 56) };
                ui.vertical(|ui| {
                    ui.set_max_width(185.0);
                    ui.add(egui::Label::new(RichText::new(title_str).size(13.5).color(title_col).strong()).wrap(true));
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.vertical(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            if let Some(ref dd) = task.due_date {
                                let short_d = if dd.chars().count() >= 5 { dd.chars().skip(5).collect::<String>() } else { dd.clone() };
                                let badge_txt = if let Some(ref orig) = task.original_due {
                                    if orig != dd && orig.chars().count() >= 5 {
                                        let orig_short: String = orig.chars().skip(5).collect();
                                        format!("{}📅{}(延)", if is_overdue { "⚠️" } else { "" }, orig_short)
                                    } else {
                                        format!("{}📅{}", if is_overdue { "⚠️" } else { "" }, short_d)
                                    }
                                } else {
                                    format!("{}📅{}", if is_overdue { "⚠️" } else { "" }, short_d)
                                };
                                let (bg, fg) = if is_overdue {
                                    (Color32::from_rgb(255, 235, 238), Color32::from_rgb(211, 47, 47))
                                } else {
                                    (Color32::from_rgb(227, 242, 253), Color32::from_rgb(21, 101, 192))
                                };
                                draw_badge(ui, &badge_txt, bg, fg);
                            }
                            if task.priority == 1 {
                                draw_badge(ui, "⭐重要", Color32::from_rgb(255, 248, 225), Color32::from_rgb(245, 127, 23));
                            }
                        });
                        ui.add_space(3.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::BOTTOM), |ui| {
                            if !rule_display.is_empty() {
                                draw_badge(ui, &format!("🔁{}", rule_display), Color32::from_rgb(232, 245, 233), Color32::from_rgb(46, 125, 50));
                            }
                            if let Some(ref tm) = task.due_time {
                                draw_badge(ui, &format!("🕒{}", tm), Color32::from_rgb(243, 229, 245), Color32::from_rgb(106, 27, 154));
                            }
                        });
                    });
                });
            });
        }).response.interact(Sense::click());

        if resp.clicked() {
            if is_pending {
                self.complete_task(&task.uuid);
            } else {
                self.pending_confirm.insert(task.uuid.clone(), Instant::now());
            }
        }

        resp.context_menu(|ui| {
            ui.set_min_width(160.0);
            if ui.button("🎯 设为当前专注目标").clicked() {
                self.current_focus_task = Some((task.uuid.clone(), task.text.clone()));
                self.current_tab = ViewTab::Forest;
                ui.close_menu();
            }
            if ui.button("➡️ 延后一天 (智能隐藏)").clicked() {
                self.postpone_task(&task.uuid);
                ui.close_menu();
            }
            if ui.button("✏️ 编辑事项详情").clicked() {
                self.open_task_editor(task);
                ui.close_menu();
            }
            ui.separator();
            if ui.button(RichText::new("🗑️ 删除此事项").color(Color32::from_rgb(211, 47, 47))).clicked() {
                self.confirm_delete_task = Some((task.uuid.clone(), task.text.clone()));
                ui.close_menu();
            }
        });
    }

    fn render_quadrant_box(&mut self, ui: &mut egui::Ui, title: &str, head_bg: Color32, body_bg: Color32, q_num: u8, today_s: &str) {
        let tasks: Vec<TodoTask> = self.todos.iter()
            .filter(|t| t.hide_until.as_ref().map(|h| today_s >= h.as_str()).unwrap_or(true))
            .filter(|t| compute_quadrant(t, today_s) == q_num)
            .cloned()
            .collect();

        egui::Frame::none()
            .fill(body_bg)
            .rounding(Rounding::same(10.0))
            .stroke(Stroke::new(1.0_f32, head_bg))
            .inner_margin(egui::Margin::same(6.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(ui.available_height());

                egui::Frame::none()
                    .fill(head_bg)
                    .rounding(Rounding::same(6.0))
                    .inner_margin(egui::Margin::symmetric(6.0, 4.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(title).size(11.5).strong().color(Color32::from_rgb(38, 50, 56)));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(format!("{}", tasks.len())).size(11.0).strong().color(Color32::from_rgb(90, 110, 90)));
                            });
                        });
                    });

                ui.add_space(4.0);

                egui::ScrollArea::vertical()
                    .id_source(format!("quad_scroll_{}", q_num))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_max_width(ui.available_width());
                    
                        if tasks.is_empty() {
                            ui.add_space(28.0);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new("暂无事项").size(11.0).color(Color32::from_rgb(175, 185, 175)));
                            });
                        }
                        for t in tasks {
                            let is_p = self.pending_confirm.contains_key(&t.uuid);
                            let mut lbl = if is_p { format!("✔️ {}", t.text) } else { format!("◯ {}", t.text) };
                            if let Some(ref tm) = t.due_time {
                                lbl.push_str(&format!(" 🕒{}", tm));
                            }
                            let col = if is_p { Color32::from_rgb(21, 101, 192) } else { Color32::from_rgb(45, 55, 60) };

                            let r = ui.add(
                                egui::Label::new(RichText::new(lbl).size(11.5).color(col))
                                    .wrap(true)
                                    .sense(Sense::click()),
                            );
                            let r = r.on_hover_text(format!("点击两次完成 | 右键更多操作\n事项：{}", t.text));
                            if r.clicked() {
                                if is_p {
                                    self.complete_task(&t.uuid);
                                } else {
                                    self.pending_confirm.insert(t.uuid.clone(), Instant::now());
                                }
                            }
                            r.context_menu(|ui| {
                                if ui.button("🎯 设为专注目标").clicked() {
                                    self.current_focus_task = Some((t.uuid.clone(), t.text.clone()));
                                    self.current_tab = ViewTab::Forest;
                                    ui.close_menu();
                                }
                                if ui.button("➡️ 延后一天").clicked() {
                                    self.postpone_task(&t.uuid);
                                    ui.close_menu();
                                }
                                if ui.button("✏️ 编辑事项").clicked() {
                                    self.open_task_editor(&t);
                                    ui.close_menu();
                                }
                                if ui.button(RichText::new("🗑️ 删除事项").color(Color32::RED)).clicked() {
                                    self.confirm_delete_task = Some((t.uuid.clone(), t.text.clone()));
                                    ui.close_menu();
                                }
                            });
                            ui.add_space(4.0);
                        }
                    });
            });
    }

    fn render_modals(&mut self, ctx: &egui::Context) {
        if self.show_popup_settings_modal {
            let mut open = true;
            egui::Window::new("🔔 休息提醒弹窗设置")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .fixed_size(Vec2::new(336.0, 430.0))
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    egui::Grid::new("popup_cfg_grid")
                        .num_columns(2)
                        .spacing([8.0, 10.0])
                        .show(ui, |ui| {
                            ui.label("弹出位置:");
                            egui::ComboBox::from_id_source("cb_popup_pos")
                                .selected_text(match self.config.popup_position.as_str() {
                                    "top_right" => "↗️ 屏幕右上角",
                                    "bottom_right" => "↘️ 屏幕右下角",
                                    "top_left" => "↖️ 屏幕左上角",
                                    "bottom_left" => "↙️ 屏幕左下角",
                                    "custom" => "📍 自定义屏幕坐标",
                                    _ => "🎯 屏幕正中央 (推荐)",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.config.popup_position, "center".into(), "🎯 屏幕正中央 (推荐)");
                                    ui.selectable_value(&mut self.config.popup_position, "top_right".into(), "↗️ 屏幕右上角");
                                    ui.selectable_value(&mut self.config.popup_position, "bottom_right".into(), "↘️ 屏幕右下角");
                                    ui.selectable_value(&mut self.config.popup_position, "top_left".into(), "↖️ 屏幕左上角");
                                    ui.selectable_value(&mut self.config.popup_position, "bottom_left".into(), "↙️ 屏幕左下角");
                                    ui.selectable_value(&mut self.config.popup_position, "custom".into(), "📍 自定义屏幕坐标");
                                });
                            ui.end_row();

                            if self.config.popup_position == "custom" {
                                ui.label("自定义坐标:");
                                ui.horizontal(|ui| {
                                    ui.label("X:");
                                    ui.add(egui::DragValue::new(&mut self.config.popup_custom_x).clamp_range(0.0..=3840.0).speed(2.0));
                                    ui.label("Y:");
                                    ui.add(egui::DragValue::new(&mut self.config.popup_custom_y).clamp_range(0.0..=2160.0).speed(2.0));
                                });
                                ui.end_row();
                            }

                            ui.label("弹窗宽高:");
                            ui.horizontal(|ui| {
                                ui.label("宽:");
                                ui.add(egui::DragValue::new(&mut self.config.popup_width).clamp_range(240.0..=700.0).speed(2.0));
                                ui.label("高:");
                                ui.add(egui::DragValue::new(&mut self.config.popup_height).clamp_range(220.0..=550.0).speed(2.0));
                            });
                            ui.end_row();

                            ui.label("弹窗标题:");
                            ui.text_edit_singleline(&mut self.config.popup_title);
                            ui.end_row();

                            ui.label("标题字号:");
                            ui.add(egui::Slider::new(&mut self.config.popup_title_size, 14.0..=36.0).suffix(" px"));
                            ui.end_row();

                            ui.label("正文字号:");
                            ui.add(egui::Slider::new(&mut self.config.popup_font_size, 12.0..=28.0).suffix(" px"));
                            ui.end_row();
                        });

                    ui.add_space(6.0);
                    ui.label("休息提示词 (支持多行回车):");
                    ui.add(egui::TextEdit::multiline(&mut self.config.popup_message).desired_rows(3).desired_width(f32::INFINITY));
                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        if ui.button("👁️ 立即预览弹窗效果").clicked() {
                            self.save_all_data();
                            self.show_popup_settings_modal = false;
                            self.trigger_rest_popup_window(ctx);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(RichText::new("💾 保存设置").strong().color(Color32::from_rgb(46, 125, 50))).clicked() {
                                self.save_all_data();
                                self.show_popup_settings_modal = false;
                                self.show_toast("✅ 休息弹窗设置已保存");
                            }
                        });
                    });
                });
            if !open {
                self.show_popup_settings_modal = false;
            }
        }

        let mut save_state = None;
        let mut close_edit = false;
        if let Some(ref mut st) = self.edit_modal {
            let win_title = if st.is_new_plan {
                "➕ 添加周期计划"
            } else if st.is_plan {
                "✏️ 编辑周期计划"
            } else {
                "✏️ 编辑事项"
            };
            egui::Window::new(win_title)
                .collapsible(false)
                .resizable(false)
                .fixed_size(Vec2::new(330.0, 340.0))
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    egui::Grid::new("edit_form_grid")
                        .num_columns(2)
                        .spacing([10.0, 10.0])
                        .show(ui, |ui| {
                            ui.label("事项名称:");
                            ui.text_edit_singleline(&mut st.title);
                            ui.end_row();

                            ui.label("优先级:");
                            ui.checkbox(&mut st.priority, "⭐ 标记为重要事项");
                            ui.end_row();

                            ui.label("截止日期:");
                            if st.is_plan {
                                ui.label(RichText::new("(由周期引擎自动派发)").size(11.5).color(Color32::GRAY));
                            } else {
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut st.has_due, "");
                                    ui.add_enabled_ui(st.has_due, |ui| {
                                        ui.add(egui::DragValue::new(&mut st.due_year).clamp_range(2024..=2035).suffix("年"));
                                        ui.add(egui::DragValue::new(&mut st.due_month).clamp_range(1..=12).suffix("月"));
                                        ui.add(egui::DragValue::new(&mut st.due_day).clamp_range(1..=31).suffix("日"));
                                    });
                                });
                            }
                            ui.end_row();

                            ui.label("截止时刻:");
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut st.has_time, "");
                                ui.add_enabled_ui(st.has_time, |ui| {
                                    ui.add(egui::DragValue::new(&mut st.time_hour).clamp_range(0..=23).suffix("时"));
                                    ui.label(":");
                                    ui.add(egui::DragValue::new(&mut st.time_min).clamp_range(0..=59).suffix("分"));
                                });
                            });
                            ui.end_row();

                            if st.is_periodic_instance {
                                ui.label("周期说明:");
                                ui.label(RichText::new("单次实例，仅改本次时间与名称").size(11.0).color(Color32::from_rgb(230, 81, 0)));
                                ui.end_row();
                            } else {
                                ui.label("重复规则:");
                                egui::ComboBox::from_id_source("cb_repeat_rule")
                                    .selected_text(match st.repeat_rule.as_str() {
                                        "daily" => "每天",
                                        "weekdays" => "工作日",
                                        "weekly" => "每周",
                                        "monthly" => "每月",
                                        _ => "不重复",
                                    })
                                    .show_ui(ui, |ui| {
                                        if !st.is_plan {
                                            ui.selectable_value(&mut st.repeat_rule, "none".to_string(), "不重复");
                                        }
                                        ui.selectable_value(&mut st.repeat_rule, "daily".to_string(), "每天");
                                        ui.selectable_value(&mut st.repeat_rule, "weekdays".to_string(), "工作日 (周一至周五)");
                                        ui.selectable_value(&mut st.repeat_rule, "weekly".to_string(), "每周 (指定周几)");
                                        ui.selectable_value(&mut st.repeat_rule, "monthly".to_string(), "每月 (指定几号)");
                                    });
                                ui.end_row();

                                if st.repeat_rule == "weekly" {
                                    ui.label("具体星期:");
                                    ui.horizontal_wrapped(|ui| {
                                        for (i, w) in ["一", "二", "三", "四", "五", "六", "日"].iter().enumerate() {
                                            ui.checkbox(&mut st.weekly_checked[i], *w);
                                        }
                                    });
                                    ui.end_row();
                                } else if st.repeat_rule == "monthly" {
                                    ui.label("具体几号:");
                                    ui.add(egui::TextEdit::singleline(&mut st.monthly_str).hint_text("如: 1, 15, 31"));
                                    ui.end_row();
                                }
                            }
                        });

                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("取消").clicked() {
                                close_edit = true;
                            }
                            if ui.button(RichText::new("💾 保存").strong().color(Color32::from_rgb(46, 125, 50))).clicked() {
                                save_state = Some(st.clone());
                            }
                        });
                    });
                });
        }
        if close_edit {
            self.edit_modal = None;
        }
        if let Some(st) = save_state {
            self.save_editor_changes(st);
        }

        if self.show_history_modal {
            let mut open = true;
            egui::Window::new("📜 已完成历史归档")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .fixed_size(Vec2::new(344.0, 550.0))
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    let total_done = self.history.len();
                    let mut grouped: BTreeMap<String, Vec<(usize, HistoryRecord)>> = BTreeMap::new();
                    for (idx, item) in self.history.iter().enumerate() {
                        let d_key: String = item.completed.chars().take(10).collect();
                        let d_key = if d_key.chars().count() == 10 { d_key } else { "未知日期".to_string() };
                        grouped.entry(d_key).or_default().push((idx, item.clone()));
                    }

                    egui::Frame::none()
                        .fill(Color32::from_rgb(232, 245, 233))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("🏆 累计已完成 {} 项任务", total_done)).size(12.5).strong().color(Color32::from_rgb(27, 94, 32)));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new("右键或点击还原可恢复").size(10.5).color(Color32::from_rgb(96, 125, 139)));
                                });
                            });
                        });
                    ui.add_space(6.0);

                    let mut restore_idx = None;
                    egui::ScrollArea::vertical()
                        .id_source("history_modal_scroll")
                        .max_height(465.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if grouped.is_empty() {
                                ui.add_space(80.0);
                                ui.vertical_centered(|ui| {
                                    ui.label(RichText::new("📭 暂无历史完成记录\n去搞定今天的第一项待办吧！").size(13.0).color(Color32::GRAY));
                                });
                            }
                            for (date_k, items) in grouped.into_iter().rev() {
                                ui.add_space(6.0);
                                egui::Frame::none()
                                    .fill(Color32::from_rgb(241, 248, 233))
                                    .rounding(Rounding::same(6.0))
                                    .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(RichText::new(format!("📅 {}  ({} 项)", date_k, items.len())).size(12.0).strong().color(Color32::from_rgb(46, 125, 50)));
                                    });
                                ui.add_space(4.0);

                                for (orig_idx, h) in items.into_iter().rev() {
                                    let t_hm: String = h.completed.chars().skip(11).take(5).collect();
                                    egui::Frame::none()
                                        .fill(Color32::WHITE)
                                        .rounding(Rounding::same(8.0))
                                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(225, 235, 225)))
                                        .inner_margin(egui::Margin::symmetric(10.0, 7.0))
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new("✔️").size(12.5).color(Color32::from_rgb(67, 160, 71)));
                                                ui.vertical(|ui| {
                                                    ui.set_max_width(225.0);
                                                    let r = ui.add(
                                                        egui::Label::new(RichText::new(&h.text).size(12.5).color(Color32::from_rgb(45, 55, 60)))
                                                            .wrap(true)
                                                            .sense(Sense::click()),
                                                    );
                                                    if !t_hm.is_empty() {
                                                        ui.label(RichText::new(format!("🕒 完成于 {}", t_hm)).size(10.5).color(Color32::GRAY));
                                                    }
                                                    r.context_menu(|ui| {
                                                        if ui.button("↩️ 还原至待办列表").clicked() {
                                                            restore_idx = Some(orig_idx);
                                                            ui.close_menu();
                                                        }
                                                    });
                                                });
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    if ui.small_button("↩️ 还原").on_hover_text("将此事项恢复到未完成待办列表").clicked() {
                                                        restore_idx = Some(orig_idx);
                                                    }
                                                });
                                            });
                                        });
                                    ui.add_space(4.0);
                                }
                            }
                        });

                    if let Some(idx) = restore_idx {
                        if idx < self.history.len() {
                            let item = self.history.remove(idx);
                            let td = today_str();
                            self.todos.insert(
                                0,
                                TodoTask {
                                    uuid: gen_uuid(),
                                    plan_uuid: item.plan_uuid,
                                    cycle_date: item.cycle_date.unwrap_or_else(|| td.clone()),
                                    text: item.text,
                                    priority: 0,
                                    due_date: item.due_date,
                                    original_due: None,
                                    due_time: None,
                                    reminder_time: None,
                                    repeat_rule: None,
                                    repeat_days: None,
                                    created: item.created.unwrap_or_else(now_iso),
                                    done: false,
                                    hide_until: None,
                                    notified: false,
                                },
                            );
                            self.sort_todos();
                            self.save_all_data();
                            self.show_toast("↩️ 已成功还原至待办列表");
                        }
                    }
                });
            if !open {
                self.show_history_modal = false;
            }
        }

        if self.show_forest_stats_modal {
            let mut open = true;
            egui::Window::new("🌲 我的专注森林档案")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .fixed_size(Vec2::new(344.0, 550.0))
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    let records = &self.cached_forest_records;
                    let healthy_cnt = records.iter().filter(|r| r.status == "healthy").count();
                    let withered_cnt = records.iter().filter(|r| r.status == "withered").count();
                    let total_mins: u32 = records.iter().filter(|r| r.status == "healthy").map(|r| r.minutes).sum();
                    let total_hours = total_mins as f32 / 60.0;
                    let total_sessions = healthy_cnt + withered_cnt;
                    let success_rate = if total_sessions > 0 { (healthy_cnt * 100) / total_sessions } else { 100 };

                    let rank_title = match healthy_cnt {
                        0 => "🌱 初入森林 · 播下第一颗绿种吧",
                        1..=4 => "🌿 护林新星 · 绿意正在悄悄萌芽",
                        5..=14 => "🌳 森林行者 · 专注已成自然习惯",
                        15..=29 => "🏞️ 绿洲领主 · 你拥有一片茂密森林",
                        _ => "👑 森林守护神 · 极致自律的掌控者",
                    };

                    egui::Frame::none()
                        .fill(Color32::from_rgb(232, 245, 233))
                        .rounding(Rounding::same(10.0))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(165, 214, 167)))
                        .inner_margin(egui::Margin::symmetric(10.0, 10.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new(rank_title).size(12.5).strong().color(Color32::from_rgb(27, 94, 32)));
                            });
                            ui.add_space(8.0);
                            let col_w = (ui.available_width() - 16.0) / 3.0;
                            ui.horizontal(|ui| {
                                egui::Frame::none().fill(Color32::WHITE).rounding(Rounding::same(8.0)).inner_margin(egui::Margin::symmetric(4.0, 6.0)).show(ui, |ui| {
                                    ui.set_width(col_w);
                                    ui.vertical_centered(|ui| {
                                        ui.label(RichText::new(format!("{} 棵", healthy_cnt)).size(17.0).strong().color(Color32::from_rgb(46, 125, 50)));
                                        ui.label(RichText::new("🌲 健康成林").size(10.5).color(Color32::GRAY));
                                    });
                                });
                                egui::Frame::none().fill(Color32::WHITE).rounding(Rounding::same(8.0)).inner_margin(egui::Margin::symmetric(4.0, 6.0)).show(ui, |ui| {
                                    ui.set_width(col_w);
                                    ui.vertical_centered(|ui| {
                                        let time_disp = if total_mins >= 60 { format!("{:.1} h", total_hours) } else { format!("{} m", total_mins) };
                                        ui.label(RichText::new(time_disp).size(17.0).strong().color(Color32::from_rgb(21, 101, 192)));
                                        ui.label(RichText::new("⏱️ 累计专注").size(10.5).color(Color32::GRAY));
                                    });
                                });
                                egui::Frame::none().fill(Color32::WHITE).rounding(Rounding::same(8.0)).inner_margin(egui::Margin::symmetric(4.0, 6.0)).show(ui, |ui| {
                                    ui.set_width(col_w);
                                    ui.vertical_centered(|ui| {
                                        ui.label(RichText::new(format!("{}%", success_rate)).size(17.0).strong().color(Color32::from_rgb(230, 81, 0)));
                                        ui.label(RichText::new(format!("🎯 成活(枯{})", withered_cnt)).size(10.5).color(Color32::GRAY));
                                    });
                                });
                            });

                            if healthy_cnt > 0 {
                                ui.add_space(8.0);
                                let mut panorama = String::new();
                                for i in 0..healthy_cnt.min(18) {
                                    panorama.push_str(if i % 3 == 0 { "🌳" } else { "🌲" });
                                }
                                if healthy_cnt > 18 {
                                    panorama.push_str(&format!(" +{}", healthy_cnt - 18));
                                }
                                ui.vertical_centered(|ui| {
                                    ui.label(RichText::new(panorama).size(14.0));
                                });
                            }
                        });

                    ui.add_space(8.0);
                    ui.label(RichText::new("📜 专注种树时光轴").size(12.5).strong().color(Color32::from_rgb(55, 71, 79)));
                    ui.add_space(4.0);

                    egui::ScrollArea::vertical()
                        .id_source("forest_stats_scroll")
                        .max_height(345.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if records.is_empty() {
                                ui.add_space(60.0);
                                ui.vertical_centered(|ui| {
                                    ui.label(RichText::new("🌱 森林还是空地，开启第一次专注种下绿树吧！").size(12.5).color(Color32::GRAY));
                                });
                            }
                            for r in records.iter().rev() {
                                let is_ok = r.status == "healthy";
                                let (bg, stroke_col, icon, title_col, badge_txt) = if is_ok {
                                    (
                                        Color32::from_rgb(246, 252, 246),
                                        Color32::from_rgb(200, 230, 201),
                                        "🌲",
                                        Color32::from_rgb(27, 94, 32),
                                        format!("+{} 分钟 🌿", r.minutes),
                                    )
                                } else {
                                    (
                                        Color32::from_rgb(255, 248, 248),
                                        Color32::from_rgb(255, 205, 210),
                                        "🥀",
                                        Color32::from_rgb(198, 40, 40),
                                        "半途枯萎".to_string(),
                                    )
                                };
                                let ts: String = r.timestamp.chars().take(16).collect();
                                egui::Frame::none()
                                    .fill(bg)
                                    .rounding(Rounding::same(8.0))
                                    .stroke(Stroke::new(1.0_f32, stroke_col))
                                    .inner_margin(egui::Margin::symmetric(10.0, 7.0))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(icon).size(20.0));
                                            ui.vertical(|ui| {
                                                ui.set_max_width(185.0);
                                                ui.label(RichText::new(&r.task).size(12.5).strong().color(title_col));
                                                ui.label(RichText::new(ts.replace('T', " ")).size(10.5).color(Color32::GRAY));
                                            });
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                draw_badge(ui, &badge_txt, Color32::WHITE, title_col);
                                            });
                                        });
                                    });
                                ui.add_space(5.0);
                            }
                        });
                });
            if !open {
                self.show_forest_stats_modal = false;
            }
        }

        if let Some((uuid, title)) = self.confirm_delete_task.clone() {
            egui::Window::new("⚠️ 确认删除事项")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(format!("确定要删除事项「{}」吗？", title));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("取消").clicked() {
                            self.confirm_delete_task = None;
                        }
                        if ui.button(RichText::new("确认删除").color(Color32::RED)).clicked() {
                            if let Some(pos) = self.todos.iter().position(|x| x.uuid == uuid) {
                                let t = self.todos.remove(pos);
                                if t.plan_uuid.is_some() {
                                    self.history.push(HistoryRecord {
                                        plan_uuid: t.plan_uuid,
                                        cycle_date: Some(t.cycle_date),
                                        text: format!("[已跳过] {}", t.text),
                                        due_date: t.due_date,
                                        created: Some(t.created),
                                        completed: now_iso(),
                                    });
                                }

                                if !self.is_focus_running {
                                    if let Some((ref focus_uuid, _)) = self.current_focus_task {
                                        if focus_uuid == &uuid {
                                            self.current_focus_task = None;
                                        }
                                    }
                                }

                                self.save_all_data();
                            }
                            self.confirm_delete_task = None;
                        }
                    });
                });
        }

        if let Some((puuid, ptitle)) = self.confirm_delete_plan.clone() {
            egui::Window::new("⚠️ 确认删除周期计划")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(format!("确定要彻底删除周期计划「{}」吗？", ptitle));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("取消").clicked() {
                            self.confirm_delete_plan = None;
                        }
                        if ui.button(RichText::new("彻底删除").color(Color32::RED)).clicked() {
                            self.plans.retain(|p| p.plan_uuid != puuid);
                            self.todos.retain(|t| t.plan_uuid.as_deref() != Some(puuid.as_str()));
                            self.save_all_data();
                            self.confirm_delete_plan = None;
                        }
                    });
                });
        }

        if self.confirm_giveup_focus {
            egui::Window::new("⚠️ 放弃专注")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("现在放弃的话，这棵正在生长的小树将会枯萎 🥀\n确定要中断专注吗？");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("继续坚持 💪").clicked() {
                            self.confirm_giveup_focus = false;
                        }
                        if ui.button(RichText::new("忍痛放弃 🥀").color(Color32::RED)).clicked() {
                            self.record_forest("withered");
                            self.reset_forest_session();
                            self.confirm_giveup_focus = false;
                        }
                    });
                });
        }
    }
}

impl eframe::App for EggDoneApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_tray_and_close_events(ctx);
        self.tick_timers(ctx);
    
        if self.is_focus_running && !self.is_resting && self.ball_icon == "🌰" {
            self.ball_icon = "🌱".to_string();
        }

        // 【优化】智能休眠引擎：运行倒计时或有弹窗时 250ms 刷新；平时 1000ms 深度休眠
        if self.is_focus_running || self.toast.is_some() || !self.pending_confirm.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }

        if self.is_rest_popup_mode {
            egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.painter().rect(
                    rect.shrink(1.0),
                    Rounding::same(16.0),
                    Color32::from_rgb(241, 248, 233),
                    Stroke::new(2.5_f32, Color32::from_rgb(76, 175, 80)),
                );
                ui.allocate_ui_at_rect(rect.shrink(18.0), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(6.0);
                        ui.label(RichText::new(&self.config.popup_title).size(self.config.popup_title_size).strong().color(Color32::from_rgb(27, 94, 32)));
                        ui.add_space(12.0);
                        ui.label(RichText::new(&self.config.popup_message).size(self.config.popup_font_size).color(Color32::from_rgb(51, 105, 30)));
                        ui.add_space(20.0);
                        ui.horizontal(|ui| {
                            let btn_w = (ui.available_width() - 12.0) * 0.5;
                            if ui.add_sized([btn_w, 38.0], egui::Button::new(RichText::new("☕ 休息 5 分钟").size(14.0).strong().color(Color32::WHITE)).fill(Color32::from_rgb(67, 160, 71))).clicked() {
                                self.start_rest_session();
                                self.collapse_window(ctx);
                            }
                            if ui.add_sized([btn_w, 38.0], egui::Button::new(RichText::new("⏭️ 跳过休息").size(13.5).color(Color32::from_rgb(55, 71, 79))).fill(Color32::WHITE)).clicked() {
                                self.reset_forest_session();
                                self.collapse_window(ctx);
                            }
                        });
                        ui.add_space(6.0);
                        if ui.small_button("🖥️ 展开主面板").clicked() {
                            self.reset_forest_session();
                            self.expand_window(ctx);
                        }
                    });
                });
            });
            return;
        }

        if !self.is_expanded {
            egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
                let (_, rect) = ui.allocate_space(Vec2::new(64.0, 64.0));
                let resp = ui.interact(rect, egui::Id::new("floating_egg_ball"), Sense::click_and_drag());

                if resp.hovered() && !ctx.input(|i| i.pointer.any_down()) {
                    if self.hover_enter_time.is_none() {
                        self.hover_enter_time = Some(Instant::now());
                    } else if self.hover_enter_time.unwrap().elapsed().as_millis() > 260 {
                        self.expand_window(ctx);
                    }
                } else {
                    self.hover_enter_time = None;
                }

                if resp.dragged_by(egui::PointerButton::Primary) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    self.has_dragged = true;
                }
              
                if ctx.input(|i| i.pointer.any_released()) && self.has_dragged {
                    if let Some(r) = ctx.input(|i| i.viewport().outer_rect) {
                        self.ball_pos = r.min;
                        self.config.ball_x = self.ball_pos.x;
                        self.config.ball_y = self.ball_pos.y;
                        safe_json_dump("config.json", &self.config);
                    }
                    self.has_dragged = false;
                }

                if resp.clicked() && !self.has_dragged {
                    self.expand_window(ctx);
                }

                resp.context_menu(|ui| {
                    ui.set_min_width(150.0);
                    if ui.button("🖥️ 展开主面板").clicked() {
                        self.expand_window(ctx);
                        ui.close_menu();
                    }
                    if ui.checkbox(&mut self.autostart_enabled, "⚡ 开机自动启动").changed() {
                        set_autostart(self.autostart_enabled);
                        if let Some(ref item) = self.tray_auto_item {
                            item.set_checked(self.autostart_enabled);
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button(RichText::new("✖ 彻底退出程序").color(Color32::from_rgb(211, 47, 47))).clicked() {
                        self.allow_quit = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                let painter = ui.painter();
                let center = rect.center();
                let bg_alpha = if resp.hovered() { 215 } else { 170 };
                let ring_alpha = if resp.hovered() { 235 } else { 190 };
                painter.circle(
                    center,
                    29.0,
                    Color32::from_rgba_unmultiplied(46, 125, 50, bg_alpha),
                    Stroke::new(2.2_f32, Color32::from_rgba_unmultiplied(255, 213, 79, ring_alpha)),
                );
                painter.circle_stroke(center, 25.0, Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 95)));

                if self.ball_time_str.is_empty() {
                    let pending_cnt = self.todos.len();
                    painter.text(center - Vec2::new(0.0, 5.0), egui::Align2::CENTER_CENTER, &self.ball_icon, egui::FontId::proportional(21.0), Color32::WHITE);
                    painter.text(center + Vec2::new(0.0, 13.0), egui::Align2::CENTER_CENTER, format!("{}项", pending_cnt), egui::FontId::proportional(10.0), Color32::from_rgb(232, 245, 233));
                } else {
                    painter.text(center - Vec2::new(0.0, 7.0), egui::Align2::CENTER_CENTER, &self.ball_icon, egui::FontId::proportional(17.0), Color32::WHITE);
                    painter.text(center + Vec2::new(0.0, 11.0), egui::Align2::CENTER_CENTER, &self.ball_time_str, egui::FontId::proportional(11.0), Color32::from_rgb(255, 245, 157));
                }
            });
            return;
        }

        let ptr_in = ctx.input(|i| i.pointer.has_pointer());
        if !ptr_in && !self.has_any_modal() && !ctx.input(|i| i.pointer.any_down()) {
            if self.mouse_leave_time.is_none() {
                self.mouse_leave_time = Some(Instant::now());
            } else if self.mouse_leave_time.unwrap().elapsed().as_millis() > 450 {
                self.collapse_window(ctx);
            }
        } else {
            self.mouse_leave_time = None;
        }

        let panel_frame = egui::Frame::none()
            .fill(Color32::from_rgb(246, 249, 246))
            .rounding(Rounding::same(14.0))
            .stroke(Stroke::new(1.5_f32, Color32::from_rgb(200, 220, 200)))
            .inner_margin(egui::Margin::same(12.0));

        egui::CentralPanel::default().frame(panel_frame).show(ctx, |ui| {
            let header_rect = ui.allocate_space(Vec2::new(ui.available_width(), 38.0)).1;
            ui.painter().rect_filled(header_rect, Rounding::same(9.0), Color32::from_rgb(46, 125, 50));
            let drag_resp = ui.interact(header_rect, egui::Id::new("header_drag_zone"), Sense::drag());
            if drag_resp.dragged_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }

            ui.allocate_ui_at_rect(header_rect.shrink2(Vec2::new(10.0, 4.0)), |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("🥚 EggDone 待办").size(15.0).strong().color(Color32::WHITE));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(RichText::new("➖").size(12.0).color(Color32::WHITE)).fill(Color32::TRANSPARENT))
                            .on_hover_text("缩回桌面悬浮球")
                            .clicked()
                        {
                            self.collapse_window(ctx);
                        }
                        if ui.add(egui::Button::new(RichText::new("🔔").size(13.0).color(Color32::WHITE)).fill(Color32::TRANSPARENT))
                            .on_hover_text("自定义休息提醒弹窗(位置/大小/提示词)")
                            .clicked()
                        {
                            self.show_popup_settings_modal = true;
                        }
                        if ui.add(egui::Button::new(RichText::new("📜").size(13.0).color(Color32::WHITE)).fill(Color32::TRANSPARENT))
                            .on_hover_text("查看已完成历史归档")
                            .clicked()
                        {
                            self.show_history_modal = true;
                        }
                        if ui.add(egui::Button::new(RichText::new("📋 交接").size(12.0).color(Color32::from_rgb(46, 125, 50))).fill(Color32::from_rgb(255, 249, 196)))
                            .on_hover_text("一键复制今明两天未完成事项用于下班交接")
                            .clicked()
                        {
                            self.copy_handover(ctx);
                        }
                    });
                });
            });

            ui.add_space(8.0);

            egui::Frame::none()
                .fill(Color32::from_rgb(230, 237, 230))
                .rounding(Rounding::same(8.0))
                .inner_margin(egui::Margin::same(3.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let tab_w = (ui.available_width() - 9.0) / 4.0;
                        for (tab, title) in [
                            (ViewTab::All, "📋 全部"),
                            (ViewTab::Quadrants, "🔲 象限"),
                            (ViewTab::Plans, "🔁 周期"),
                            (ViewTab::Forest, "🌲 森林"),
                        ] {
                            let selected = self.current_tab == tab;
                            let bg = if selected { Color32::WHITE } else { Color32::TRANSPARENT };
                            let fg = if selected { Color32::from_rgb(46, 125, 50) } else { Color32::from_rgb(96, 125, 139) };
                            let btn = egui::Button::new(RichText::new(title).size(12.5).strong().color(fg))
                                .fill(bg)
                                .stroke(Stroke::NONE)
                                .rounding(Rounding::same(6.0));
                            if ui.add_sized([tab_w, 27.0], btn).clicked() {
                                self.current_tab = tab;
                            }
                        }
                    });
                });

            ui.add_space(8.0);
          
            // 【优化】直接从内存缓存读取当下的日期和时间，告别每帧创建 String 对象
            let today_s = &self.cached_today_str;
            let now_hm = &self.cached_now_hm;

            match self.current_tab {
                ViewTab::All | ViewTab::Quadrants => {
                    egui::Frame::none()
                        .fill(Color32::WHITE)
                        .rounding(Rounding::same(8.0))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(200, 230, 201)))
                        .inner_margin(egui::Margin::symmetric(8.0, 5.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("✍️").size(14.0));
                                let edit = egui::TextEdit::singleline(&mut self.input_text)
                                    .frame(false)
                                    .hint_text("输入如: 明天15点交报表 !重要 #每天 (回车添加)")
                                    .desired_width(ui.available_width() - 36.0);
                                let resp = ui.add(edit);
                                if resp.changed() {
                                    self.parsed_preview = QuickAddParser::parse(&self.input_text)
                                        .map(|p| {
                                            let mut tip = format!("✨ 标题: {}", p.title);
                                            if p.priority == 1 {
                                                tip.push_str(" | ⭐重要");
                                            }
                                            if let Some(ref d) = p.due_date {
                                                tip.push_str(&format!(" | 📅{}", d));
                                            }
                                            if let Some(ref t) = p.due_time {
                                                tip.push_str(&format!(" 🕒{}", t));
                                            }
                                            if let Some(ref r) = p.repeat_rule {
                                                tip.push_str(&format!(" | 🔁{}", r));
                                            }
                                            tip
                                        })
                                        .unwrap_or_default();
                                }
                                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                    self.commit_new_task();
                                    resp.request_focus();
                                }
                                if ui.add(egui::Button::new(RichText::new("➕").color(Color32::WHITE)).fill(Color32::from_rgb(67, 160, 71))).clicked() {
                                    self.commit_new_task();
                                }
                            });
                        });

                    if !self.parsed_preview.is_empty() {
                        ui.add_space(2.0);
                        ui.label(RichText::new(&self.parsed_preview).size(11.0).color(Color32::from_rgb(46, 125, 50)));
                    }
                    ui.add_space(6.0);

                    if self.current_tab == ViewTab::All {
                        let visible_tasks: Vec<(TodoTask, String)> = self.todos.iter()
                            .filter(|t| t.hide_until.as_ref().map(|h| today_s.as_str() >= h.as_str()).unwrap_or(true))
                            .map(|t| {
                                let r_str = if let Some(ref pu) = t.plan_uuid {
                                    if let Some(p) = self.plans.iter().find(|x| &x.plan_uuid == pu) {
                                        get_rule_display_str(Some(&p.repeat_rule), Some(&p.repeat_days))
                                    } else {
                                        get_rule_display_str(t.repeat_rule.as_deref(), t.repeat_days.as_ref())
                                    }
                                } else {
                                    get_rule_display_str(t.repeat_rule.as_deref(), t.repeat_days.as_ref())
                                };
                                (t.clone(), r_str)
                            })
                            .collect();

                        egui::ScrollArea::vertical().id_source("all_tasks_scroll").show(ui, |ui| {
                            if visible_tasks.is_empty() {
                                ui.add_space(70.0);
                                ui.vertical_centered(|ui| {
                                    ui.label(RichText::new("🎉 太棒了！当前所有待办已清空").size(13.5).color(Color32::from_rgb(120, 144, 156)));
                                });
                            }
                            for (task, rule_disp) in visible_tasks {
                                self.render_task_card(ui, &task, &rule_disp, today_s, now_hm);
                                ui.add_space(6.0);
                            }
                        });
                    } else {
                        let avail_h = (ui.available_height() - 8.0).max(200.0);
                        let half_h = (avail_h - 8.0) * 0.5;
                        let half_w = (ui.available_width() - 8.0) * 0.5;

                        ui.horizontal(|ui| {
                            ui.allocate_ui(Vec2::new(half_w, half_h), |ui| {
                                self.render_quadrant_box(ui, "🔥 重要且紧急", Color32::from_rgb(255, 205, 210), Color32::from_rgb(255, 248, 248), 1, today_s);
                            });
                            ui.allocate_ui(Vec2::new(half_w, half_h), |ui| {
                                self.render_quadrant_box(ui, "⭐ 重要不紧急", Color32::from_rgb(255, 224, 178), Color32::from_rgb(255, 252, 245), 2, today_s);
                            });
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.allocate_ui(Vec2::new(half_w, half_h), |ui| {
                                self.render_quadrant_box(ui, "⚡ 紧急不重要", Color32::from_rgb(187, 222, 251), Color32::from_rgb(245, 250, 255), 3, today_s);
                            });
                            ui.allocate_ui(Vec2::new(half_w, half_h), |ui| {
                                self.render_quadrant_box(ui, "🌱 不重要不紧急", Color32::from_rgb(200, 230, 201), Color32::from_rgb(246, 252, 246), 4, today_s);
                            });
                        });
                    }
                }

                ViewTab::Plans => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("💡 自动按周期派发今日待办").size(12.0).color(Color32::from_rgb(96, 125, 139)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new(RichText::new("➕ 新建周期计划").size(12.0).color(Color32::WHITE)).fill(Color32::from_rgb(46, 125, 50))).clicked() {
                                self.open_plan_editor(None);
                            }
                        });
                    });
                    ui.add_space(6.0);
                    let plans_snap = self.plans.clone();
                    egui::ScrollArea::vertical().id_source("plans_scroll").show(ui, |ui| {
                        if plans_snap.is_empty() {
                            ui.add_space(60.0);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new("暂无周期计划，点击右上角新建 🔁").color(Color32::GRAY));
                            });
                        }
                        for p in plans_snap {
                            let r_txt = get_rule_display_str(Some(&p.repeat_rule), Some(&p.repeat_days));
                            egui::Frame::none()
                                .fill(Color32::WHITE)
                                .rounding(Rounding::same(10.0))
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(215, 230, 215)))
                                .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| {
                                            ui.set_max_width(210.0);
                                            ui.label(RichText::new(&p.text).size(13.5).strong());
                                            ui.add_space(3.0);
                                            ui.horizontal(|ui| {
                                                draw_badge(ui, &format!("🔁 {}", r_txt), Color32::from_rgb(232, 245, 233), Color32::from_rgb(46, 125, 50));
                                                if let Some(ref tm) = p.due_time {
                                                    draw_badge(ui, &format!("🕒 {}", tm), Color32::from_rgb(243, 229, 245), Color32::from_rgb(106, 27, 154));
                                                }
                                                if p.priority == 1 {
                                                    draw_badge(ui, "⭐重要", Color32::from_rgb(255, 248, 225), Color32::from_rgb(245, 127, 23));
                                                }
                                            });
                                        });
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.button("🗑️").on_hover_text("删除此周期计划").clicked() {
                                                self.confirm_delete_plan = Some((p.plan_uuid.clone(), p.text.clone()));
                                            }
                                            if ui.button("✏️").on_hover_text("编辑此周期计划").clicked() {
                                                self.open_plan_editor(Some(&p));
                                            }
                                        });
                                    });
                                });
                            ui.add_space(6.0);
                        }
                    });
                }

                ViewTab::Forest => {
                    egui::Frame::none()
                        .fill(Color32::WHITE)
                        .rounding(Rounding::same(12.0))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(200, 230, 201)))
                        .inner_margin(egui::Margin::same(14.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical_centered(|ui| {
                                ui.horizontal(|ui| {
                                    let target_str = self.current_focus_task.as_ref().map(|(_, t)| format!("🎯 当前目标: {}", t)).unwrap_or_else(|| "🌱 自由专注模式 (右键任务可绑定)".to_string());
                                    ui.label(RichText::new(target_str).size(12.5).strong().color(Color32::from_rgb(46, 125, 50)));
                                    if self.current_focus_task.is_some() && !self.is_focus_running && ui.small_button("✖").clicked() {
                                        self.current_focus_task = None;
                                    }
                                });

                                ui.add_space(16.0);
                                let tree_emoji = if self.is_focus_running {
                                    if self.ball_icon == "🌰" { "🌱" } else { self.ball_icon.as_str() }
                                } else {
                                    "🌲"
                                };
                                egui::Frame::none()
                                    .fill(Color32::from_rgb(232, 245, 233))
                                    .rounding(Rounding::same(50.0))
                                    .inner_margin(egui::Margin::symmetric(22.0, 12.0))
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(tree_emoji).size(62.0).color(Color32::from_rgb(46, 125, 50)));
                                    });
                                ui.add_space(8.0);

                                let m = self.remaining_secs / 60;
                                let s = self.remaining_secs % 60;
                                ui.label(RichText::new(format!("{:02}:{:02}", m, s)).size(42.0).strong().color(Color32::from_rgb(27, 94, 32)));
                                ui.add_space(10.0);

                                if !self.is_focus_running {
                                    ui.horizontal(|ui| {
                                        ui.label("专注时长:");
                                        if ui.add(egui::Slider::new(&mut self.focus_minutes, 5..=120).step_by(5.0).suffix(" 分钟")).changed() {
                                            self.remaining_secs = self.focus_minutes * 60;
                                            self.config.focus_minutes = self.focus_minutes;
                                            safe_json_dump("config.json", &self.config);
                                        }
                                    });
                                    ui.add_space(12.0);
                                    if ui.add_sized([200.0, 40.0], egui::Button::new(RichText::new("🌱 开始种树专注").size(15.0).strong().color(Color32::WHITE)).fill(Color32::from_rgb(67, 160, 71))).clicked() {
                                        self.is_resting = false;
                                        self.remaining_secs = self.focus_minutes * 60;
                                        self.is_focus_running = true;
                                        self.last_sec_tick = Instant::now();
                                        self.ball_icon = "🌱".to_string();
                                    }
                                } else {
                                    ui.add_space(12.0);
                                    if self.is_resting {
                                        if ui.add_sized([180.0, 38.0], egui::Button::new(RichText::new("⏭️ 结束休息").size(14.0).color(Color32::WHITE)).fill(Color32::from_rgb(2, 136, 209))).clicked() {
                                            self.reset_forest_session();
                                        }
                                    } else if ui.add_sized([180.0, 38.0], egui::Button::new(RichText::new("🥀 放弃专注").size(14.0).color(Color32::WHITE)).fill(Color32::from_rgb(229, 57, 53))).clicked() {
                                        self.confirm_giveup_focus = true;
                                    }
                                }

                                ui.add_space(16.0);
                                ui.separator();
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    if ui.button(RichText::new("🏆 森林统计档案").strong().color(Color32::from_rgb(46, 125, 50))).clicked() {
                                        self.cached_forest_records = safe_json_load("forest.json");
                                        self.show_forest_stats_modal = true;
                                    }
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.button("🔔 休息弹窗设置").clicked() {
                                            self.show_popup_settings_modal = true;
                                        }
                                    });
                                });
                            });
                        });
                }
            }
        });

        self.render_modals(ctx);

        if let Some((msg, _)) = &self.toast {
            egui::Area::new(egui::Id::new("bottom_floating_toast"))
                .order(egui::Order::Foreground)
                .interactable(false)
                .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -20.0))
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(Color32::from_rgba_unmultiplied(38, 105, 42, 238))
                        .rounding(Rounding::same(20.0))
                        .stroke(Stroke::new(1.2_f32, Color32::from_rgb(165, 214, 167)))
                        .inner_margin(egui::Margin::symmetric(16.0, 8.0))
                        .show(ui, |ui| {
                            ui.label(RichText::new(msg).size(12.5).strong().color(Color32::WHITE));
                        });
                });
        }
    }
}

fn draw_badge(ui: &mut egui::Ui, text: &str, bg: Color32, fg: Color32) {
    egui::Frame::none()
        .fill(bg)
        .rounding(Rounding::same(4.0))
        .inner_margin(egui::Margin::symmetric(5.0, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.5).color(fg).strong());
        });
}

fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let font_candidates = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyhbd.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/System/Library/Fonts/PingFang.ttc",
    ];
    for path in font_candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert("cjk".to_owned(), FontData::from_owned(bytes));
            fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "cjk".to_owned());
            fonts.families.entry(FontFamily::Monospace).or_default().push("cjk".to_owned());
            break;
        }
    }
    let emoji_candidates = [
        "C:\\Windows\\Fonts\\seguiemj.ttf",
        "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    ];
    for path in emoji_candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert("emoji".to_owned(), FontData::from_owned(bytes));
            fonts.families.entry(FontFamily::Proportional).or_default().push("emoji".to_owned());
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn main() -> eframe::Result<()> {
    let instance = single_instance::SingleInstance::new("EggDone_Desktop_SingleInstance_Lock_v2").unwrap();
    if !instance.is_single() {
        return Ok(());
    }

    let cfg: Config = safe_json_load("config.json");
    let init_pos = Pos2::new(cfg.ball_x.max(10.0), cfg.ball_y.max(10.0));

    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([64.0, 64.0])
        .with_position(init_pos)
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false)
        .with_resizable(false);

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "EggDone",
        options,
        Box::new(move |cc| Box::new(EggDoneApp::new(cc, init_pos))),
    )
}
