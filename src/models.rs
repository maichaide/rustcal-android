use chrono::{Datelike, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Calendar {
    pub id: String,
    pub name: String,
    pub color: String,
    pub account_type: String,
    pub is_visible: bool,
    pub sync_url: Option<String>,
}

/// 日程重要/紧急程度分类（对应月视图颜色横条）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventUrgency {
    ImportantUrgent,    // 红色：重要紧急
    ImportantNotUrgent, // 紫色：重要但不紧急
    NormalNotUrgent,    // 蓝色：一般且不紧急
    NormalUrgent,       // 黄色：一般但紧急
    ReminderOnly,       // 黑色：仅供提醒
}

impl EventUrgency {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventUrgency::ImportantUrgent => "IMPORTANT_URGENT",
            EventUrgency::ImportantNotUrgent => "IMPORTANT_NOT_URGENT",
            EventUrgency::NormalNotUrgent => "NORMAL_NOT_URGENT",
            EventUrgency::NormalUrgent => "NORMAL_URGENT",
            EventUrgency::ReminderOnly => "REMINDER_ONLY",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_uppercase().as_str() {
            "IMPORTANT_URGENT" => EventUrgency::ImportantUrgent,
            "IMPORTANT_NOT_URGENT" => EventUrgency::ImportantNotUrgent,
            "NORMAL_NOT_URGENT" => EventUrgency::NormalNotUrgent,
            "NORMAL_URGENT" => EventUrgency::NormalUrgent,
            "REMINDER_ONLY" => EventUrgency::ReminderOnly,
            _ => EventUrgency::NormalNotUrgent,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            EventUrgency::ImportantUrgent => "🔴 重要且紧急 (红)",
            EventUrgency::ImportantNotUrgent => "🟣 重要但不紧急 (紫)",
            EventUrgency::NormalNotUrgent => "🔵 一般且不紧急 (蓝)",
            EventUrgency::NormalUrgent => "🟡 一般但紧急 (黄)",
            EventUrgency::ReminderOnly => "⚫ 仅供提醒 (黑)",
        }
    }

    pub fn bar_hex(&self) -> &'static str {
        match self {
            EventUrgency::ImportantUrgent => "#DC2626",    // 红色
            EventUrgency::ImportantNotUrgent => "#7C3AED", // 紫色
            EventUrgency::NormalNotUrgent => "#2563EB",    // 蓝色
            EventUrgency::NormalUrgent => "#EAB308",       // 黄色
            EventUrgency::ReminderOnly => "#111827",       // 黑色
        }
    }

    pub fn all() -> [EventUrgency; 5] {
        [
            EventUrgency::ImportantUrgent,
            EventUrgency::ImportantNotUrgent,
            EventUrgency::NormalNotUrgent,
            EventUrgency::NormalUrgent,
            EventUrgency::ReminderOnly,
        ]
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Recurrence {
    None,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Recurrence {
    pub fn as_str(&self) -> &'static str {
        match self {
            Recurrence::None => "NONE",
            Recurrence::Daily => "DAILY",
            Recurrence::Weekly => "WEEKLY",
            Recurrence::Monthly => "MONTHLY",
            Recurrence::Yearly => "YEARLY",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "DAILY" => Recurrence::Daily,
            "WEEKLY" => Recurrence::Weekly,
            "MONTHLY" => Recurrence::Monthly,
            "YEARLY" => Recurrence::Yearly,
            _ => Recurrence::None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Recurrence::None => "不重复",
            Recurrence::Daily => "每天",
            Recurrence::Weekly => "每周",
            Recurrence::Monthly => "每月",
            Recurrence::Yearly => "每年",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub calendar_id: String,
    pub summary: String,
    pub description: String,
    pub start_time: NaiveDateTime,
    pub end_time: NaiveDateTime,
    pub is_all_day: bool,
    pub recurrence: Recurrence,
    pub exception_dates: Vec<NaiveDate>,
    pub reminder_minutes: Option<i64>,
    #[serde(default = "default_urgency")]
    pub urgency: EventUrgency,
}

fn default_urgency() -> EventUrgency {
    EventUrgency::NormalNotUrgent
}

impl Event {
    pub fn occurs_on(&self, date: NaiveDate) -> bool {
        if self.exception_dates.contains(&date) {
            return false;
        }
        let start_date = self.start_time.date();
        let end_date = self.end_time.date();
        if date < start_date {
            return false;
        }
        match self.recurrence {
            Recurrence::None => date >= start_date && date <= end_date,
            Recurrence::Daily => true,
            Recurrence::Weekly => date.weekday() == start_date.weekday(),
            Recurrence::Monthly => date.day() == start_date.day(),
            Recurrence::Yearly => date.month() == start_date.month() && date.day() == start_date.day(),
        }
    }

    pub fn conflicts_with(&self, other: &Event, date: NaiveDate) -> bool {
        if self.id == other.id || self.is_all_day || other.is_all_day {
            return false;
        }
        if !self.occurs_on(date) || !other.occurs_on(date) {
            return false;
        }
        let s1 = self.start_time.time();
        let e1 = self.end_time.time();
        let s2 = other.start_time.time();
        let e2 = other.end_time.time();
        s1 < e2 && s2 < e1
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: String,
    pub calendar_id: String,
    pub title: String,
    pub description: String,
    pub priority: u8,
    pub completed: bool,
    pub due_date: Option<NaiveDate>,
}

/// 界面前景字体色与背景色配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub fg_color: String,
    pub bg_color: String,
    pub month_cell_transparent: bool,
    pub month_cell_bg_color: String,
    pub month_date_fg_color: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            fg_color: "#000000".to_string(),
            bg_color: "#FFFFFF".to_string(),
            month_cell_transparent: true,
            month_cell_bg_color: "#F8FAFC".to_string(),
            month_date_fg_color: "#000000".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupBundle {
    pub version: String,
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
    pub todos: Vec<Todo>,
}
