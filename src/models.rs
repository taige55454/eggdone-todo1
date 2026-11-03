use chrono::{Datelike, Duration, Local, NaiveDate};
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
}
fn default_title() -> String { "待办".to_string() }
fn default_focus_mins() -> u32 { 25 }
impl Default for Config {
    fn default() -> Self { Self { title_prefix: default_title(), focus_minutes: default_focus_mins() } }
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
    fn cn_to_num(s: &str) -> u32 {
        if let Ok(n) = s.parse::<u32>() { return n; }
        let map_c = |c: char| match c {
            '一' => 1, '二' | '两' => 2, '三' => 3, '四' => 4, '五' => 5,
            '六' => 6, '七' => 7, '八' => 8, '九' => 9, '十' => 10, _ => 0,
        };
        let cs: Vec<char> = s.chars().collect();
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
            '四' => Some(3), '五' => Some(4), '六' => Some(5), '日' | '天' => Some(6), _ => None,
        }
    }

    pub fn parse(raw_text: &str) -> Option<ParsedTask> {
        let mut text = raw_text.trim().to_string();
        if text.is_empty() { return None; }
        let (mut priority, mut target_date, mut due_time, mut repeat_rule) = (0, None, None, None);
        let (mut repeat_days, mut labels, mut base_date_override) = (Vec::new(), Vec::new(), None);

        static RE_PRIO: OnceLock<Regex> = OnceLock::new();
        let re_prio = RE_PRIO.get_or_init(|| Regex::new(r"(?i)^([!！]|important\b)\s*").unwrap());
        if let Some(m) = re_prio.find(&text) {
            priority = 1;
            labels.push("⭐ 重要".into());
            text = text[m.end()..].trim().to_string();
        } else if text.contains("重要") {
            priority = 1;
            labels.push("⭐ 重要".into());
            text = text.replacen("重要", " ", 1).trim().to_string();
        }

        let today = Local::now().date_naive();
        static RE_TOM: OnceLock<Regex> = OnceLock::new();
        static RE_TOD: OnceLock<Regex> = OnceLock::new();
        let re_tom = RE_TOM.get_or_init(|| Regex::new(r"(?i)(明天|tomorrow\b)").unwrap());
        let re_tod = RE_TOD.get_or_init(|| Regex::new(r"(?i)(今天|today\b)").unwrap());

        if text.contains("大后天") {
            target_date = Some(today + Duration::days(3));
            labels.push("大后天".into());
            text = text.replacen("大后天", "", 1).trim().to_string();
        } else if text.contains("后天") {
            target_date = Some(today + Duration::days(2));
            labels.push("后天".into());
            text = text.replacen("后天", "", 1).trim().to_string();
        } else if let Some(m) = re_tom.find(&text) {
            target_date = Some(today + Duration::days(1));
            labels.push("明天".into());
            text.replace_range(m.range(), "");
            text = text.trim().to_string();
        } else if let Some(m) = re_tod.find(&text) {
            target_date = Some(today);
            labels.push("今天".into());
            text.replace_range(m.range(), "");
            text = text.trim().to_string();
        }

        static RE_DAILY: OnceLock<Regex> = OnceLock::new();
        static RE_WKDAYS: OnceLock<Regex> = OnceLock::new();
        static RE_WEEKLY: OnceLock<Regex> = OnceLock::new();
        static RE_MONTHLY: OnceLock<Regex> = OnceLock::new();
        let re_daily = RE_DAILY.get_or_init(|| Regex::new(r"(?i)(每天|daily\b)").unwrap());
        let re_wkdays = RE_WKDAYS.get_or_init(|| Regex::new(r"(?i)(工作日|weekdays\b)").unwrap());
        let re_weekly = RE_WEEKLY.get_or_init(|| Regex::new(r"每周([一二三四五六日天和、,，\s]+)").unwrap());
        let re_monthly = RE_MONTHLY.get_or_init(|| Regex::new(r"每月([0-3]?\d(?:[、,，和\s]+[0-3]?\d)*)[号日]").unwrap());

        if let Some(m) = re_daily.find(&text) {
            repeat_rule = Some("daily".into());
            labels.push("🔁 每天".into());
            text.replace_range(m.range(), "");
            text = text.trim().to_string();
        } else if let Some(m) = re_wkdays.find(&text) {
            repeat_rule = Some("weekdays".into());
            labels.push("🔁 工作日".into());
            text.replace_range(m.range(), "");
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
            text.replace_range(caps.get(0).unwrap().range(), "");
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
            let num_re = Regex::new(r"\d+").unwrap();
            let mut nums: Vec<u32> = num_re.find_iter(caps.get(1).unwrap().as_str())
                .filter_map(|m| m.as_str().parse::<u32>().ok())
                .filter(|&n| (1..=31).contains(&n))
                .collect();
            nums.sort_unstable();
            nums.dedup();
            if nums.is_empty() { nums.push(1); }
            repeat_days = nums;
            labels.push(format!("🔁 每月{}日", repeat_days.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(",")));
            text.replace_range(caps.get(0).unwrap().range(), "");
            text = text.trim().to_string();
            target_date = (0..32).map(|i| today + Duration::days(i)).find(|ed| {
                let max_d = days_in_month(ed.year(), ed.month());
                repeat_days.iter().any(|&d| d.min(max_d) == ed.day())
            }).or(Some(today));
            base_date_override = target_date.map(|d| d.format("%Y-%m-%d").to_string());
        }

        if target_date.is_none() && repeat_rule.is_none() {
            static RE_WK: OnceLock<Regex> = OnceLock::new();
            let re_wk = RE_WK.get_or_init(|| Regex::new(r"(下)?(?:周|星期)([一二三四五六日天])").unwrap());
            if let Some(caps) = re_wk.captures(&text) {
                let is_next = caps.get(1).is_some();
                let target_wk = Self::weekday_map(caps.get(2).unwrap().as_str().chars().next().unwrap()).unwrap_or(0);
                let cur_wk = today.weekday().num_days_from_monday();
                let mut ahead = (target_wk as i64 - cur_wk as i64 + 7) % 7;
                if ahead == 0 && !is_next { ahead = 7; }
                if is_next { ahead += 7; }
                target_date = Some(today + Duration::days(ahead));
                labels.push(caps.get(0).unwrap().as_str().to_string());
                text.replace_range(caps.get(0).unwrap().range(), "");
                text = text.trim().to_string();
            }
        }

        static RE_TM: OnceLock<Regex> = OnceLock::new();
        static RE_CH_TM: OnceLock<Regex> = OnceLock::new();
        let re_tm = RE_TM.get_or_init(|| Regex::new(r"\b([0-2]?\d)[:：]([0-5]\d)\b").unwrap());
        let re_ch_tm = RE_CH_TM.get_or_init(|| Regex::new(r"(上午|下午|晚上)?([0-2]?\d|[零一二两三四五六七八九十]+)点(?:(半)|([0-5]?\d|[零一二三四五六七八九十]+)分)?").unwrap());

        if let Some(caps) = re_tm.captures(&text) {
            let h: u32 = caps.get(1).unwrap().as_str().parse().unwrap_or(0);
            let m: u32 = caps.get(2).unwrap().as_str().parse().unwrap_or(0);
            if h < 24 && m < 60 {
                due_time = Some(format!("{:02}:{:02}", h, m));
                labels.push(format!("🕒 {:02}:{:02}", h, m));
                text.replace_range(caps.get(0).unwrap().range(), "");
                text = text.trim().to_string();
            }
        } else if let Some(caps) = re_ch_tm.captures(&text) {
            let ampm = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let mut h = Self::cn_to_num(caps.get(2).unwrap().as_str());
            let m = if caps.get(3).is_some() { 30 } else { caps.get(4).map(|x| Self::cn_to_num(x.as_str())).unwrap_or(0) };
            if (ampm == "下午" || ampm == "晚上") && h < 12 { h += 12; }
            if ampm == "上午" && h == 12 { h = 0; }
            if h < 24 && m < 60 {
                due_time = Some(format!("{:02}:{:02}", h, m));
                labels.push(format!("🕒 {:02}:{:02}", h, m));
                text.replace_range(caps.get(0).unwrap().range(), "");
                text = text.trim().to_string();
            }
        }

        if due_time.is_some() && target_date.is_none() { target_date = Some(today); }
        let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let (title, labels, priority) = if clean.is_empty() { (raw_text.trim().to_string(), Vec::new(), 0) } else { (clean, labels, priority) };
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
// === models.rs 文件结束 ===
