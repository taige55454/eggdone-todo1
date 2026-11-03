use chrono::{Datelike, Duration, Local, NaiveDate, Timelike};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::SystemTime;
use uuid::Uuid;

#[cfg(windows)]
pub fn is_autostart_enabled() -> bool {
    use winreg::enums::*;
    use winreg::RegKey;
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Run", KEY_READ)
        .and_then(|k| k.get_value::<String, _>("EggDoneTodo"))
        .is_ok()
}
#[cfg(not(windows))]
pub fn is_autostart_enabled() -> bool { false }

#[cfg(windows)]
pub fn set_autostart(enable: bool) -> bool {
    use winreg::enums::*;
    use winreg::RegKey;
    if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Run", KEY_SET_VALUE) {
        if enable {
            if let Ok(exe) = env::current_exe() {
                return key.set_value("EggDoneTodo", &format!("\"{}\"", exe.to_string_lossy())).is_ok();
            }
        } else {
            let _ = key.delete_value("EggDoneTodo");
            return true;
        }
    }
    false
}
#[cfg(not(windows))]
pub fn set_autostart(_enable: bool) -> bool { false }

pub fn get_base_dir() -> PathBuf {
    env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())).unwrap_or_else(|| PathBuf::from("."))
}

pub fn safe_json_load<T: for<'de> Deserialize<'de> + Default>(filename: &str) -> T {
    let base = get_base_dir();
    for f in [base.join(filename), base.join(format!("{}.bak", filename))] {
        if let Ok(content) = fs::read_to_string(&f) {
            if let Ok(data) = serde_json::from_str::<T>(&content) {
                return data;
            }
        }
    }
    T::default()
}

pub fn safe_json_dump<T: Serialize + ?Sized>(filename: &str, data: &T) {
    let base = get_base_dir();
    let path = base.join(filename);
    let tmp_path = base.join(format!("{}.tmp", filename));
    let bak_path = base.join(format!("{}.bak", filename));

    if path.exists() {
        let need_bak = fs::metadata(&bak_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|mtime| SystemTime::now().duration_since(mtime).ok())
            .map(|d| d.as_secs() > 86400)
            .unwrap_or(true);
        if need_bak {
            let _ = fs::copy(&path, &bak_path);
        }
    }
    if let Ok(json_str) = serde_json::to_string_pretty(data) {
        if fs::write(&tmp_path, &json_str).is_ok() {
            if fs::rename(&tmp_path, &path).is_err() {
                let _ = fs::write(&path, &json_str);
                let _ = fs::remove_file(&tmp_path);
            }
        } else {
            let _ = fs::write(&path, &json_str);
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    #[serde(default = "default_title")]
    pub title_prefix: String,
    #[serde(default = "default_focus_mins")]
    pub focus_minutes: u32,
    #[serde(default = "default_ball_x")]
    pub ball_x: f32,
    #[serde(default = "default_ball_y")]
    pub ball_y: f32,
    #[serde(default = "default_popup_pos")]
    pub popup_position: String, // "center", "top_right", "bottom_right", "top_left", "bottom_left", "custom"
    #[serde(default = "default_popup_x")]
    pub popup_custom_x: f32,
    #[serde(default = "default_popup_y")]
    pub popup_custom_y: f32,
    #[serde(default = "default_popup_w")]
    pub popup_width: f32,
    #[serde(default = "default_popup_h")]
    pub popup_height: f32,
    #[serde(default = "default_popup_title")]
    pub popup_title: String,
    #[serde(default = "default_popup_msg")]
    pub popup_message: String,
    #[serde(default = "default_popup_title_size")]
    pub popup_title_size: f32,
    #[serde(default = "default_popup_font_size")]
    pub popup_font_size: f32,
}

fn default_title() -> String { "待办".to_string() }
fn default_focus_mins() -> u32 { 25 }
fn default_ball_x() -> f32 { -1.0 }
fn default_ball_y() -> f32 { 80.0 }
fn default_popup_pos() -> String { "center".to_string() }
fn default_popup_x() -> f32 { 500.0 }
fn default_popup_y() -> f32 { 300.0 }
fn default_popup_w() -> f32 { 320.0 }
fn default_popup_h() -> f32 { 300.0 }
fn default_popup_title() -> String { "专注完成！".to_string() }
fn default_popup_msg() -> String { "路漫漫其修远兮\n起身喝杯水、远眺休息一下吧！".to_string() }
fn default_popup_title_size() -> f32 { 22.0 }
fn default_popup_font_size() -> f32 { 15.0 }

impl Default for Config {
    fn default() -> Self {
        Self {
            title_prefix: default_title(),
            focus_minutes: default_focus_mins(),
            ball_x: default_ball_x(),
            ball_y: default_ball_y(),
            popup_position: default_popup_pos(),
            popup_custom_x: default_popup_x(),
            popup_custom_y: default_popup_y(),
            popup_width: default_popup_w(),
            popup_height: default_popup_h(),
            popup_title: default_popup_title(),
            popup_message: default_popup_msg(),
            popup_title_size: default_popup_title_size(),
            popup_font_size: default_popup_font_size(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TodoTask {
    #[serde(default = "gen_uuid")]
    pub uuid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_uuid: Option<String>,
    #[serde(default = "today_str")]
    pub cycle_date: String,
    pub text: String,
    #[serde(default)]
    pub priority: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_due: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_days: Option<Vec<u32>>,
    #[serde(default = "now_iso")]
    pub created: String,
    #[serde(default)]
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hide_until: Option<String>,
    #[serde(default)]
    pub notified: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Plan {
    pub plan_uuid: String,
    pub text: String,
    #[serde(default)]
    pub priority: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder_time: Option<String>,
    pub repeat_rule: String,
    #[serde(default)]
    pub repeat_days: Vec<u32>,
    pub base_date: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HistoryRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle_date: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    pub completed: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ForestRecord {
    pub timestamp: String,
    pub status: String,
    pub task: String,
    pub minutes: u32,
}

pub fn gen_uuid() -> String { Uuid::new_v4().to_string() }
pub fn today_str() -> String { Local::now().format("%Y-%m-%d").to_string() }
pub fn now_iso() -> String { Local::now().format("%Y-%m-%dT%H:%M:%S").to_string() }

pub struct ParsedTask {
    pub title: String,
    pub priority: u8,
    pub due_date: Option<String>,
    pub due_time: Option<String>,
    pub repeat_rule: Option<String>,
    pub repeat_days: Vec<u32>,
    pub base_date: Option<String>,
    pub labels: Vec<String>,
}

pub struct QuickAddParser;
impl QuickAddParser {
    pub fn cn_to_num(s: &str) -> u32 {
        if let Ok(n) = s.trim().parse::<u32>() { return n; }
        let map_c = |c: char| match c {
            '零' | '〇' => 0, '一' | '元' | '正' => 1, '二' | '两' => 2, '三' => 3, '四' => 4,
            '五' => 5, '六' => 6, '七' => 7, '八' => 8, '九' => 9, '十' => 10, _ => 0,
        };
        let cs: Vec<char> = s.trim().chars().collect();
        match cs.len() {
            1 => map_c(cs[0]),
            2 if cs[0] == '十' => 10 + map_c(cs[1]),
            2 if cs[1] == '十' => map_c(cs[0]) * 10,
            3 if cs[1] == '十' => map_c(cs[0]) * 10 + map_c(cs[2]),
            _ => 0,
        }
    }

    fn weekday_map(c: char) -> Option<u32> {
        match c {
            '一' => Some(0), '二' | '两' => Some(1), '三' => Some(2),
            '四' => Some(3), '五' => Some(4), '六' => Some(5), '日' | '天' | '末' => Some(6), _ => None,
        }
    }

    pub fn parse(raw_text: &str) -> Option<ParsedTask> {
        let mut text = raw_text.trim().to_string();
        if text.is_empty() { return None; }
        let (mut priority, mut target_date, mut due_time, mut repeat_rule) = (0, None, None, None);
        let (mut repeat_days, mut labels, mut base_date_override) = (Vec::new(), Vec::new(), None);
        let mut implied_pm = false;
        let mut implied_morning = false;

        // 1. 优先级识别 (! / 重要 / 紧急)
        static RE_PRIO: OnceLock<Regex> = OnceLock::new();
        let re_prio = RE_PRIO.get_or_init(|| Regex::new(r"(?i)^([!！]+|important\b|紧急|重要)\s*").unwrap());
        if let Some(m) = re_prio.find(&text) {
            priority = 1;
            labels.push("⭐ 重要".into());
            text = text[m.end()..].trim().to_string();
        } else if text.contains("重要") || text.contains("紧急") {
            priority = 1;
            labels.push("⭐ 重要".into());
            text = text.replacen("重要", " ", 1).replacen("紧急", " ", 1).trim().to_string();
        }

        let now = Local::now();
        let today = now.date_naive();

        // 2. 复合相对词 (今晚/明早/明晚/后天早上等)
        let compound_rules = [
            ("大后天晚上", 3, true, false, "大后天"),
            ("大后天早上", 3, false, true, "大后天"),
            ("大后天上午", 3, false, false, "大后天"),
            ("大后天下午", 3, true, false, "大后天"),
            ("后天晚上", 2, true, false, "后天"),
            ("后天早上", 2, false, true, "后天"),
            ("后天上午", 2, false, false, "后天"),
            ("后天下午", 2, true, false, "后天"),
            ("明天晚上", 1, true, false, "明天"),
            ("明天早上", 1, false, true, "明天"),
            ("明天上午", 1, false, false, "明天"),
            ("明天下午", 1, true, false, "明天"),
            ("明晚", 1, true, false, "明天"),
            ("明早", 1, false, true, "明天"),
            ("明晨", 1, false, true, "明天"),
            ("今天晚上", 0, true, false, "今天"),
            ("今天早上", 0, false, true, "今天"),
            ("今天上午", 0, false, false, "今天"),
            ("今天下午", 0, true, false, "今天"),
            ("今晚", 0, true, false, "今天"),
            ("今早", 0, false, true, "今天"),
            ("今晨", 0, false, true, "今天"),
        ];
        for (kw, days, is_pm, is_morn, lbl) in compound_rules {
            if text.contains(kw) {
                target_date = Some(today + Duration::days(days));
                implied_pm = is_pm;
                implied_morning = is_morn;
                labels.push(format!("📅 {}", lbl));
                text = text.replacen(kw, " ", 1).trim().to_string();
                break;
            }
        }

        // 3. 基础相对日期 (今天/明天/后天/大后天/月底/月初)
        if target_date.is_none() {
            static RE_TOM: OnceLock<Regex> = OnceLock::new();
            static RE_TOD: OnceLock<Regex> = OnceLock::new();
            let re_tom = RE_TOM.get_or_init(|| Regex::new(r"(?i)(明天|明日|tomorrow\b|tmr\b)").unwrap());
            let re_tod = RE_TOD.get_or_init(|| Regex::new(r"(?i)(今天|今日|today\b)").unwrap());

            if text.contains("大后天") {
                target_date = Some(today + Duration::days(3));
                labels.push("📅 大后天".into());
                text = text.replacen("大后天", " ", 1).trim().to_string();
            } else if text.contains("后天") {
                target_date = Some(today + Duration::days(2));
                labels.push("📅 后天".into());
                text = text.replacen("后天", " ", 1).trim().to_string();
            } else if let Some(m) = re_tom.find(&text) {
                target_date = Some(today + Duration::days(1));
                labels.push("📅 明天".into());
                text.replace_range(m.range(), " ");
                text = text.trim().to_string();
            } else if let Some(m) = re_tod.find(&text) {
                target_date = Some(today);
                labels.push("📅 今天".into());
                text.replace_range(m.range(), " ");
                text = text.trim().to_string();
            } else if text.contains("月底") || text.contains("月末") {
                let max_d = days_in_month(today.year(), today.month());
                let d = NaiveDate::from_ymd_opt(today.year(), today.month(), max_d).unwrap_or(today);
                target_date = Some(d);
                labels.push(format!("📅 月底({:02}-{:02})", d.month(), d.day()));
                text = text.replacen("月底", " ", 1).replacen("月末", " ", 1).trim().to_string();
            } else if text.contains("下月初") {
                let (ny, nm) = if today.month() == 12 { (today.year() + 1, 1) } else { (today.year(), today.month() + 1) };
                let d = NaiveDate::from_ymd_opt(ny, nm, 1).unwrap_or(today);
                target_date = Some(d);
                labels.push(format!("📅 {:02}-01", nm));
                text = text.replacen("下月初", " ", 1).trim().to_string();
            }
        }

        // 4. 相对时间偏移: X天后 / X周后 / X小时后 / X分钟后 / 半小时后
        static RE_OFFSET_DAY: OnceLock<Regex> = OnceLock::new();
        static RE_OFFSET_MIN: OnceLock<Regex> = OnceLock::new();
        let re_off_day = RE_OFFSET_DAY.get_or_init(|| Regex::new(r"(\d+|[一二两三四五六七八九十]+)\s*(天|日|周|星期|礼拜)(?:后|之后|以后)").unwrap());
        let re_off_min = RE_OFFSET_MIN.get_or_init(|| Regex::new(r"(?:(半)\s*小时|(\d+|[一二两三四五六七八九十]+)\s*(?:个)?(小时|钟头|分钟|分))(?:后|之后|以后)").unwrap());

        if target_date.is_none() {
            if let Some(caps) = re_off_day.captures(&text) {
                let n = Self::cn_to_num(caps.get(1).unwrap().as_str()) as i64;
                let unit = caps.get(2).unwrap().as_str();
                let days = if unit == "天" || unit == "日" { n } else { n * 7 };
                let d = today + Duration::days(days);
                target_date = Some(d);
                labels.push(format!("📅 {:02}-{:02}", d.month(), d.day()));
                text.replace_range(caps.get(0).unwrap().range(), " ");
                text = text.trim().to_string();
            }
        }
        if let Some(caps) = re_off_min.captures(&text) {
            let mins = if caps.get(1).is_some() {
                30
            } else {
                let n = Self::cn_to_num(caps.get(2).unwrap().as_str()) as i64;
                let u = caps.get(3).unwrap().as_str();
                if u == "小时" || u == "钟头" { n * 60 } else { n }
            };
            let future = now + Duration::minutes(mins);
            target_date = Some(future.date_naive());
            let tm_str = future.format("%H:%M").to_string();
            due_time = Some(tm_str.clone());
            labels.push(format!("📅 {:02}-{:02}", future.month(), future.day()));
            labels.push(format!("🕒 {}", tm_str));
            text.replace_range(caps.get(0).unwrap().range(), " ");
            text = text.trim().to_string();
        }

        // 5. 周期性规则识别 (每天 / 工作日 / 每周X / 每月X号)
        static RE_DAILY: OnceLock<Regex> = OnceLock::new();
        static RE_WKDAYS: OnceLock<Regex> = OnceLock::new();
        static RE_WEEKLY: OnceLock<Regex> = OnceLock::new();
        static RE_MONTHLY: OnceLock<Regex> = OnceLock::new();
        let re_daily = RE_DAILY.get_or_init(|| Regex::new(r"(?i)(每天|每日|daily\b)").unwrap());
        let re_wkdays = RE_WKDAYS.get_or_init(|| Regex::new(r"(?i)(每个?工作日|weekdays\b)").unwrap());
        let re_weekly = RE_WEEKLY.get_or_init(|| Regex::new(r"每(?:周|星期|礼拜)([一二三四五六日天末和、,，\s]+)").unwrap());
        let re_monthly = RE_MONTHLY.get_or_init(|| Regex::new(r"每月([0-3]?\d|[一二三四五六七八九十]+)(?:[、,，和\s]+([0-3]?\d|[一二三四五六七八九十]+))*[号日]?").unwrap());

        if let Some(m) = re_daily.find(&text) {
            repeat_rule = Some("daily".into());
            labels.push("🔁 每天".into());
            text.replace_range(m.range(), " ");
            text = text.trim().to_string();
        } else if let Some(m) = re_wkdays.find(&text) {
            repeat_rule = Some("weekdays".into());
            labels.push("🔁 工作日".into());
            text.replace_range(m.range(), " ");
            text = text.trim().to_string();
        }

        if let Some(caps) = re_weekly.captures(&text) {
            repeat_rule = Some("weekly".into());
            let mut days: Vec<u32> = caps.get(1).unwrap().as_str().chars().filter_map(Self::weekday_map).collect();
            days.sort_unstable();
            days.dedup();
            if days.is_empty() { days.push(today.weekday().num_days_from_monday()); }
            repeat_days = days;
            let w_names = ["一", "二", "三", "四", "五", "六", "日"];
            let l_str: String = repeat_days.iter().map(|&d| w_names[d as usize]).collect();
            labels.push(format!("🔁 每周{}", l_str));
            text.replace_range(caps.get(0).unwrap().range(), " ");
            text = text.trim().to_string();
            for i in 0..7 {
                let ed = today + Duration::days(i);
                if repeat_days.contains(&ed.weekday().num_days_from_monday()) {
                    target_date = Some(ed);
                    break;
                }
            }
        }

        if let Some(caps) = re_monthly.captures(&text) {
            repeat_rule = Some("monthly".into());
            let full_match = caps.get(0).unwrap().as_str();
            let token_re = Regex::new(r"\d+|[一二三四五六七八九十]+").unwrap();
            let mut nums: Vec<u32> = token_re.find_iter(full_match)
                .map(|m| Self::cn_to_num(m.as_str()))
                .filter(|&n| (1..=31).contains(&n))
                .collect();
            nums.sort_unstable();
            nums.dedup();
            if nums.is_empty() { nums.push(1); }
            repeat_days = nums;
            labels.push(format!("🔁 每月{}日", repeat_days.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(",")));
            text.replace_range(caps.get(0).unwrap().range(), " ");
            text = text.trim().to_string();
            target_date = (0..32).map(|i| today + Duration::days(i)).find(|ed| {
                let max_d = days_in_month(ed.year(), ed.month());
                repeat_days.iter().any(|&d| d.min(max_d) == ed.day())
            }).or(Some(today));
            base_date_override = target_date.map(|d| d.format("%Y-%m-%d").to_string());
        }

        // 6. 具体日期识别 (支持: 2026年11月6日 / 11月6日 / 十一月六号 / 11-6 / 12号 / 十二日)
        if target_date.is_none() && repeat_rule.is_none() {
            static RE_YMD_CN: OnceLock<Regex> = OnceLock::new();
            static RE_YMD_DASH: OnceLock<Regex> = OnceLock::new();
            static RE_MD_CN: OnceLock<Regex> = OnceLock::new();
            static RE_MD_DASH: OnceLock<Regex> = OnceLock::new();
            static RE_DAY_ONLY: OnceLock<Regex> = OnceLock::new();

            let re_ymd_cn = RE_YMD_CN.get_or_init(|| Regex::new(r"(20\d{2}|\d{2})年\s*([01]?\d|[一二三四五六七八九十]+)月\s*([0-3]?\d|[一二三四五六七八九十]+)[号日]?").unwrap());
            let re_ymd_dash = RE_YMD_DASH.get_or_init(|| Regex::new(r"\b(20\d{2})[-/.]([01]?\d)[-/.]([0-3]?\d)\b").unwrap());
            let re_md_cn = RE_MD_CN.get_or_init(|| Regex::new(r"([01]?\d|[一二三四五六七八九十]+|本|当|下个?|下)月\s*([0-3]?\d|[一二三四五六七八九十]+)[号日]?").unwrap());
            let re_md_dash = RE_MD_DASH.get_or_init(|| Regex::new(r"\b([01]?\d)[-/.]([0-3]?\d)\b").unwrap());
            let re_day_only = RE_DAY_ONLY.get_or_init(|| Regex::new(r"([0-3]?\d|[一二三四五六七八九十]+)[号日]").unwrap());

            if let Some(caps) = re_ymd_cn.captures(&text) {
                let mut y: i32 = caps.get(1).unwrap().as_str().parse().unwrap_or(today.year());
                if y < 100 { y += 2000; }
                let m = Self::cn_to_num(caps.get(2).unwrap().as_str());
                let d = Self::cn_to_num(caps.get(3).unwrap().as_str());
                if (1..=12).contains(&m) && (1..=days_in_month(y, m)).contains(&d) {
                    if let Some(nd) = NaiveDate::from_ymd_opt(y, m, d) {
                        target_date = Some(nd);
                        labels.push(format!("📅 {}", nd.format("%Y-%m-%d")));
                        text.replace_range(caps.get(0).unwrap().range(), " ");
                        text = text.trim().to_string();
                    }
                }
            } else if let Some(caps) = re_ymd_dash.captures(&text) {
                let y: i32 = caps.get(1).unwrap().as_str().parse().unwrap_or(today.year());
                let m: u32 = caps.get(2).unwrap().as_str().parse().unwrap_or(0);
                let d: u32 = caps.get(3).unwrap().as_str().parse().unwrap_or(0);
                if (1..=12).contains(&m) && (1..=days_in_month(y, m)).contains(&d) {
                    if let Some(nd) = NaiveDate::from_ymd_opt(y, m, d) {
                        target_date = Some(nd);
                        labels.push(format!("📅 {}", nd.format("%Y-%m-%d")));
                        text.replace_range(caps.get(0).unwrap().range(), " ");
                        text = text.trim().to_string();
                    }
                }
            } else if let Some(caps) = re_md_cn.captures(&text) {
                let m_str = caps.get(1).unwrap().as_str();
                let mut y = today.year();
                let m = if m_str == "本" || m_str == "当" {
                    today.month()
                } else if m_str.starts_with('下') {
                    if today.month() == 12 { y += 1; 1 } else { today.month() + 1 }
                } else {
                    Self::cn_to_num(m_str)
                };
                let d = Self::cn_to_num(caps.get(2).unwrap().as_str());
                if (1..=12).contains(&m) {
                    let max_d = days_in_month(y, m);
                    if (1..=max_d).contains(&d) {
                        if let Some(nd) = NaiveDate::from_ymd_opt(y, m, d) {
                            target_date = Some(nd);
                            labels.push(format!("📅 {:02}-{:02}", m, d));
                            text.replace_range(caps.get(0).unwrap().range(), " ");
                            text = text.trim().to_string();
                        }
                    }
                }
            } else if let Some(caps) = re_day_only.captures(&text) {
                let d = Self::cn_to_num(caps.get(1).unwrap().as_str());
                if (1..=31).contains(&d) {
                    let mut y = today.year();
                    let mut m = today.month();
                    if d < today.day() {
                        if m == 12 { y += 1; m = 1; } else { m += 1; }
                    }
                    let actual_d = d.min(days_in_month(y, m));
                    if let Some(nd) = NaiveDate::from_ymd_opt(y, m, actual_d) {
                        target_date = Some(nd);
                        labels.push(format!("📅 {:02}-{:02}", m, actual_d));
                        text.replace_range(caps.get(0).unwrap().range(), " ");
                        text = text.trim().to_string();
                    }
                }
            } else if let Some(caps) = re_md_dash.captures(&text) {
                let m: u32 = caps.get(1).unwrap().as_str().parse().unwrap_or(0);
                let d: u32 = caps.get(2).unwrap().as_str().parse().unwrap_or(0);
                let y = today.year();
                if (1..=12).contains(&m) && (1..=days_in_month(y, m)).contains(&d) {
                    if let Some(nd) = NaiveDate::from_ymd_opt(y, m, d) {
                        target_date = Some(nd);
                        labels.push(format!("📅 {:02}-{:02}", m, d));
                        text.replace_range(caps.get(0).unwrap().range(), " ");
                        text = text.trim().to_string();
                    }
                }
            }
        }

        // 7. 星期几 / 周末识别 (本周X / 下周X / 周末)
        if target_date.is_none() && repeat_rule.is_none() {
            static RE_WK: OnceLock<Regex> = OnceLock::new();
            let re_wk = RE_WK.get_or_init(|| Regex::new(r"(本|这|下个?|下)?(?:周|星期|礼拜)([一二三四五六日天末])").unwrap());
            if let Some(caps) = re_wk.captures(&text) {
                let prefix = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let w_char = caps.get(2).unwrap().as_str().chars().next().unwrap();
                let target_wk = if w_char == '末' { 5 } else { Self::weekday_map(w_char).unwrap_or(0) };
                let cur_wk = today.weekday().num_days_from_monday();
                let d = if prefix.starts_with('下') {
                    let days_to_next_mon = (7 - cur_wk as i64) % 7;
                    let next_mon = today + Duration::days(if days_to_next_mon == 0 { 7 } else { days_to_next_mon });
                    next_mon + Duration::days(target_wk as i64)
                } else {
                    let mut ahead = (target_wk as i64 - cur_wk as i64 + 7) % 7;
                    if ahead == 0 && prefix.is_empty() { ahead = 7; }
                    today + Duration::days(ahead)
                };
                target_date = Some(d);
                labels.push(format!("📅 {}({:02}-{:02})", caps.get(0).unwrap().as_str(), d.month(), d.day()));
                text.replace_range(caps.get(0).unwrap().range(), " ");
                text = text.trim().to_string();
            } else if text.contains("周末") {
                let cur_wk = today.weekday().num_days_from_monday();
                let ahead = if cur_wk < 5 { 5 - cur_wk as i64 } else { 0 };
                let d = today + Duration::days(ahead);
                target_date = Some(d);
                labels.push(format!("📅 周末({:02}-{:02})", d.month(), d.day()));
                text = text.replacen("周末", " ", 1).trim().to_string();
            }
        }

        // 8. 具体时刻识别 (14:30 / 下午3点半 / 晚上8点15分 / 中午12点)
        if due_time.is_none() {
            static RE_TM: OnceLock<Regex> = OnceLock::new();
            static RE_CH_TM: OnceLock<Regex> = OnceLock::new();
            let re_tm = RE_TM.get_or_init(|| Regex::new(r"(清晨|早上|早晨|上午|中午|午后|下午|傍晚|晚上|夜里|深夜|凌晨)?\s*\b([0-2]?\d)[:：]([0-5]\d)\b").unwrap());
            let re_ch_tm = RE_CH_TM.get_or_init(|| Regex::new(r"(清晨|早上|早晨|上午|中午|午后|下午|傍晚|晚上|夜里|深夜|凌晨)?\s*([0-2]?\d|[零一二两三四五六七八九十]+)\s*[点时](?:(半)|([一三])刻|([0-5]?\d|[零一二三四五六七八九十]+)\s*分?)?").unwrap());

            if let Some(caps) = re_tm.captures(&text) {
                let ampm = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let mut h: u32 = caps.get(2).unwrap().as_str().parse().unwrap_or(0);
                let m: u32 = caps.get(3).unwrap().as_str().parse().unwrap_or(0);
                if ( matches!(ampm, "下午" | "傍晚" | "晚上" | "夜里" | "午后") || implied_pm ) && h < 12 { h += 12; }
                if ( matches!(ampm, "凌晨" | "早上" | "上午") || implied_morning ) && h == 12 { h = 0; }
                if h < 24 && m < 60 {
                    due_time = Some(format!("{:02}:{:02}", h, m));
                    labels.push(format!("🕒 {:02}:{:02}", h, m));
                    text.replace_range(caps.get(0).unwrap().range(), " ");
                    text = text.trim().to_string();
                }
            } else if let Some(caps) = re_ch_tm.captures(&text) {
                let ampm = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let mut h = Self::cn_to_num(caps.get(2).unwrap().as_str());
                let m = if caps.get(3).is_some() {
                    30
                } else if let Some(q) = caps.get(4) {
                    if q.as_str() == "一" { 15 } else { 45 }
                } else {
                    caps.get(5).map(|x| Self::cn_to_num(x.as_str())).unwrap_or(0)
                };
                if ( matches!(ampm, "下午" | "傍晚" | "晚上" | "夜里" | "午后") || implied_pm ) && h < 12 { h += 12; }
                if ampm == "中午" && h < 11 { h += 12; }
                if ( matches!(ampm, "凌晨" | "早上" | "上午") || implied_morning ) && h == 12 { h = 0; }
                if h < 24 && m < 60 {
                    due_time = Some(format!("{:02}:{:02}", h, m));
                    labels.push(format!("🕒 {:02}:{:02}", h, m));
                    text.replace_range(caps.get(0).unwrap().range(), " ");
                    text = text.trim().to_string();
                }
            }
        }

        // 若有隐含早晚但没写具体几点，给默认时刻
        if due_time.is_none() {
            if implied_pm {
                due_time = Some("20:00".into());
                labels.push("🕒 20:00".into());
            } else if implied_morning {
                due_time = Some("09:00".into());
                labels.push("🕒 09:00".into());
            }
        }

        // 若只写了时刻但没写日期，智能判断今天还是明天
        if let Some(ref tm) = due_time {
            if target_date.is_none() {
                let cur_hm = now.format("%H:%M").to_string();
                if tm.as_str() < cur_hm.as_str() && now.hour() >= 20 {
                    target_date = Some(today + Duration::days(1));
                } else {
                    target_date = Some(today);
                }
            }
        }

        let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let (title, labels, priority) = if clean.is_empty() {
            (raw_text.trim().to_string(), Vec::new(), 0)
        } else {
            (clean, labels, priority)
        };
        let due_date = target_date.map(|d| d.format("%Y-%m-%d").to_string());
        let base_date = base_date_override.or_else(|| due_date.clone());

        Some(ParsedTask { title, priority, due_date, due_time, repeat_rule, repeat_days, base_date, labels })
    }
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    let next = if month == 12 { NaiveDate::from_ymd_opt(year + 1, 1, 1) } else { NaiveDate::from_ymd_opt(year, month + 1, 1) };
    next.unwrap().pred_opt().unwrap().day()
}

pub fn compute_quadrant(todo: &TodoTask, today_str: &str) -> u8 {
    let imp = todo.priority == 1;
    let urg = todo.due_date.as_ref().map(|d| d.as_str() <= today_str).unwrap_or(todo.due_time.is_some());
    match (imp, urg) { (true, true) => 1, (true, false) => 2, (false, true) => 3, (false, false) => 4 }
}

pub fn get_rule_display_str(rule: Option<&str>, days: Option<&Vec<u32>>) -> String {
    match rule {
        Some("daily") => "每天".into(),
        Some("weekdays") => "工作日".into(),
        Some("weekly") => {
            if let Some(dl) = days.filter(|x| !x.is_empty()) {
                let wn = ["一", "二", "三", "四", "五", "六", "日"];
                return format!("每周{}", dl.iter().filter_map(|&d| wn.get(d as usize)).copied().collect::<String>());
            }
            "每周".into()
        }
        Some("monthly") => {
            if let Some(dl) = days.filter(|x| !x.is_empty()) {
                return format!("每月{}日", dl.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(","));
            }
            "每月".into()
        }
        Some(_) => "周期".into(),
        None => "".into(),
    }
}
// === models.rs 彻底结束 ===
