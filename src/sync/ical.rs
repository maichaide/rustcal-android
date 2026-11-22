use crate::models::{BackupBundle, Calendar, Event, EventUrgency, Recurrence, Todo};
use anyhow::{Context, Result};
use chardetng::EncodingDetector;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use encoding_rs::{Encoding, GB18030, UTF_16BE, UTF_16LE, UTF_8};
use std::path::Path;
use uuid::Uuid;

/// 针对 RFC 5545 字节流先按物理字节展开折行（CRLF + 空格/制表符），
/// 防止多字节中文（UTF-8 3字节 / GBK 2字节）在 75 字节折行处被截断导致解码乱码！
fn unfold_raw_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r'
            && i + 2 < bytes.len()
            && bytes[i + 1] == b'\n'
            && (bytes[i + 2] == b' ' || bytes[i + 2] == b'\t')
        {
            i += 3;
            continue;
        } else if bytes[i] == b'\n'
            && i + 1 < bytes.len()
            && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t')
        {
            i += 2;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// 去除 UTF-8 / UTF-16 BOM
fn strip_bom_str(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

/// 检查解码结果是否包含有效的 iCalendar 结构关键字（防止 UTF-16 误判导致全篇中文乱码）
fn looks_like_valid_ics_or_text(s: &str) -> bool {
    let head: String = s.chars().take(1024).collect();
    head.contains("BEGIN:VCALENDAR")
        || head.contains("BEGIN:VEVENT")
        || head.contains("VERSION:2.0")
        || head.trim_start().starts_with('{')
}

/// 智能多语言文本解码：支持 UTF-8、UTF-16LE/BE、GB18030/GBK、Big5、Shift_JIS、EUC-KR 等
pub fn read_text_file_multilingual(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("无法读取文件: {}", path.display()))?;
    Ok(decode_bytes_multilingual(&bytes))
}

pub fn decode_bytes_multilingual(raw_bytes: &[u8]) -> String {
    // 1. 优先检查明确的 BOM 头 (UTF-8 / UTF-16LE / UTF-16BE)
    if let Some((enc, bom_len)) = Encoding::for_bom(raw_bytes) {
        let (cow, _, _) = enc.decode(&raw_bytes[bom_len..]);
        let unfolded = unfold_raw_bytes(cow.as_bytes());
        return strip_bom_str(&String::from_utf8_lossy(&unfolded)).to_owned();
    }

    // 先对单字节/UTF-8流执行字节级折行展开，避免汉字在折行边界被劈开
    let bytes = unfold_raw_bytes(raw_bytes);

    // 2. 尝试标准 UTF-8 严格解码（绝大多数现代日历默认导出格式）
    if let Ok(s) = std::str::from_utf8(&bytes) {
        return strip_bom_str(s).to_owned();
    }

    // 3. 尝试无 BOM 的 UTF-16LE / UTF-16BE
    if raw_bytes.len() >= 8 && raw_bytes.len() % 2 == 0 {
        let (cow_le, _, err_le) = UTF_16LE.decode(raw_bytes);
        if !err_le && looks_like_valid_ics_or_text(&cow_le) {
            return strip_bom_str(&cow_le).to_owned();
        }
        let (cow_be, _, err_be) = UTF_16BE.decode(raw_bytes);
        if !err_be && looks_like_valid_ics_or_text(&cow_be) {
            return strip_bom_str(&cow_be).to_owned();
        }
    }

    // 4. 尝试 GB18030（完整超集兼容 GBK / GB2312 中文编码）
    let (cow_gb, _, err_gb) = GB18030.decode(&bytes);
    if !err_gb {
        return strip_bom_str(&cow_gb).to_owned();
    }

    // 5. 使用 chardetng 统计学检测多语言编码
    let mut detector = EncodingDetector::new();
    detector.feed(&bytes, true);
    let detected = detector.guess(None, true);
    let (cow, _, had_errors) = detected.decode(&bytes);
    if !had_errors {
        return strip_bom_str(&cow).to_owned();
    }

    // 6. 兜底容错解码
    let (fallback, _, _) = UTF_8.decode(&bytes);
    strip_bom_str(&fallback).to_owned()
}

/// 展开文本行
fn unfold_ics_lines(content: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in content.split('\n') {
        let line = raw.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix(' ').or_else(|| line.strip_prefix('\t')) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(line.to_string());
    }
    lines
}

/// 导出为标准 RFC 5545 iCalendar (.ics) 字符串（含 VEVENT 与 VTODO）
pub fn export_to_ics(events: &[Event], todos: &[Todo]) -> String {
    let mut out = String::new();
    out.push_str("BEGIN:VCALENDAR\r\n");
    out.push_str("VERSION:2.0\r\n");
    out.push_str("PRODID:-//RustCal//Calcurse GUI Replacement//EN\r\n");
    out.push_str("CALSCALE:GREGORIAN\r\n");

    for ev in events {
        out.push_str("BEGIN:VEVENT\r\n");
        out.push_str(&format!("UID:{}\r\n", ev.id));
        out.push_str(&format!("SUMMARY:{}\r\n", escape_ics(&ev.summary)));
        if !ev.description.is_empty() {
            out.push_str(&format!("DESCRIPTION:{}\r\n", escape_ics(&ev.description)));
        }
        out.push_str(&format!("X-RUSTCAL-URGENCY:{}\r\n", ev.urgency.as_str()));
        if ev.is_all_day {
            out.push_str(&format!(
                "DTSTART;VALUE=DATE:{}\r\n",
                ev.start_time.format("%Y%m%d")
            ));
            out.push_str(&format!(
                "DTEND;VALUE=DATE:{}\r\n",
                ev.end_time.format("%Y%m%d")
            ));
        } else {
            out.push_str(&format!(
                "DTSTART:{}\r\n",
                ev.start_time.format("%Y%m%dT%H%M%S")
            ));
            out.push_str(&format!(
                "DTEND:{}\r\n",
                ev.end_time.format("%Y%m%dT%H%M%S")
            ));
        }
        if ev.recurrence != Recurrence::None {
            out.push_str(&format!("RRULE:FREQ={}\r\n", ev.recurrence.as_str()));
        }
        for ex in &ev.exception_dates {
            out.push_str(&format!("EXDATE;VALUE=DATE:{}\r\n", ex.format("%Y%m%d")));
        }
        out.push_str("END:VEVENT\r\n");
    }

    for td in todos {
        out.push_str("BEGIN:VTODO\r\n");
        out.push_str(&format!("UID:{}\r\n", td.id));
        out.push_str(&format!("SUMMARY:{}\r\n", escape_ics(&td.title)));
        if !td.description.is_empty() {
            out.push_str(&format!("DESCRIPTION:{}\r\n", escape_ics(&td.description)));
        }
        out.push_str(&format!("PRIORITY:{}\r\n", td.priority));
        out.push_str(&format!(
            "STATUS:{}\r\n",
            if td.completed { "COMPLETED" } else { "NEEDS-ACTION" }
        ));
        if let Some(due) = td.due_date {
            out.push_str(&format!("DUE;VALUE=DATE:{}\r\n", due.format("%Y%m%d")));
        }
        out.push_str("END:VTODO\r\n");
    }

    out.push_str("END:VCALENDAR\r\n");
    out
}

/// 从 .ics 字符串解析 VEVENT 与 VTODO
pub fn import_from_ics_with_progress<F>(
    content: &str,
    default_calendar_id: &str,
    mut on_progress: F,
) -> Result<(Vec<Event>, Vec<Todo>)>
where
    F: FnMut(usize, usize),
{
    let lines = unfold_ics_lines(content);
    let total_lines = lines.len().max(1);

    let mut events = Vec::new();
    let mut todos = Vec::new();

    #[derive(PartialEq)]
    enum Block {
        None,
        Event,
        Todo,
        Alarm, // 隔离 VALARM 子组件，避免内部属性干扰主日程
    }

    let mut mode = Block::None;
    let mut uid = String::new();
    let mut summary = String::new();
    let mut description = String::new();
    let mut dtstart: Option<NaiveDateTime> = None;
    let mut dtend: Option<NaiveDateTime> = None;
    let mut is_all_day = false;
    let mut recurrence = Recurrence::None;
    let mut exception_dates = Vec::new();
    let mut urgency = EventUrgency::NormalNotUrgent;
    let mut priority: u8 = 3;
    let mut completed = false;
    let mut due_date: Option<NaiveDate> = None;

    for (idx, raw_line) in lines.iter().enumerate() {
        if idx % 200 == 0 {
            on_progress(idx, total_lines);
        }

        let line = raw_line.trim();
        if line == "BEGIN:VEVENT" {
            mode = Block::Event;
            uid = Uuid::new_v4().to_string();
            summary.clear();
            description.clear();
            dtstart = None;
            dtend = None;
            is_all_day = false;
            recurrence = Recurrence::None;
            exception_dates.clear();
            urgency = EventUrgency::NormalNotUrgent;
            continue;
        } else if line == "BEGIN:VALARM" && mode == Block::Event {
            mode = Block::Alarm;
            continue;
        } else if line == "END:VALARM" && mode == Block::Alarm {
            mode = Block::Event;
            continue;
        } else if line == "END:VEVENT" && mode == Block::Event {
            let start = dtstart.unwrap_or_else(|| chrono::Local::now().naive_local());
            let end = dtend.unwrap_or(start);
            events.push(Event {
                id: uid.clone(),
                calendar_id: default_calendar_id.to_string(),
                summary: if summary.is_empty() {
                    "(无标题日程)".into()
                } else {
                    summary.clone()
                },
                description: description.clone(),
                start_time: start,
                end_time: end,
                is_all_day,
                recurrence,
                exception_dates: exception_dates.clone(),
                reminder_minutes: None, // 彻底取消导入时的默认提前提醒
                urgency,
            });
            mode = Block::None;
            continue;
        } else if line == "BEGIN:VTODO" {
            mode = Block::Todo;
            uid = Uuid::new_v4().to_string();
            summary.clear();
            description.clear();
            priority = 3;
            completed = false;
            due_date = None;
            continue;
        } else if line == "END:VTODO" && mode == Block::Todo {
            todos.push(Todo {
                id: uid.clone(),
                calendar_id: default_calendar_id.to_string(),
                title: if summary.is_empty() {
                    "(无标题任务)".into()
                } else {
                    summary.clone()
                },
                description: description.clone(),
                priority,
                completed,
                due_date,
            });
            mode = Block::None;
            continue;
        }

        if mode == Block::None || mode == Block::Alarm {
            continue;
        }

        if let Some((key_part, val)) = line.split_once(':') {
            let key = key_part.split(';').next().unwrap_or("");
            match key {
                "UID" => uid = val.trim().to_string(),
                "SUMMARY" => summary = unescape_ics(val),
                "DESCRIPTION" => description = unescape_ics(val),
                "X-RUSTCAL-URGENCY" => urgency = EventUrgency::from_str(val),
                "PRIORITY" => {
                    let p = val.trim().parse::<u8>().unwrap_or(5);
                    priority = p.clamp(1, 9);
                    if p <= 2 {
                        urgency = EventUrgency::ImportantUrgent;
                    } else if p <= 4 {
                        urgency = EventUrgency::ImportantNotUrgent;
                    }
                }
                "DTSTART" => {
                    if let Some((dt, all_day)) = parse_ics_datetime(val) {
                        dtstart = Some(dt);
                        is_all_day = all_day;
                    }
                }
                "DTEND" => {
                    if let Some((dt, _)) = parse_ics_datetime(val) {
                        dtend = Some(dt);
                    }
                }
                "RRULE" => {
                    let upper_val = val.trim().to_uppercase();
                    // 过滤掉仅执行 1 次的伪重复规则（如 FREQ=DAILY;COUNT=1）
                    let is_single_count = upper_val
                        .split(';')
                        .any(|p| p.trim() == "COUNT=1");

                    if !is_single_count {
                        for part in upper_val.split(';') {
                            if let Some(freq) = part.trim().strip_prefix("FREQ=") {
                                // 仅保留显式合法的每周/每月/每年重复，过滤掉异常的每日重复
                                recurrence = match freq {
                                    "WEEKLY" => Recurrence::Weekly,
                                    "MONTHLY" => Recurrence::Monthly,
                                    "YEARLY" => Recurrence::Yearly,
                                    _ => Recurrence::None,
                                };
                            }
                        }
                    }
                }
                "EXDATE" => {
                    if let Some((dt, _)) = parse_ics_datetime(val) {
                        exception_dates.push(dt.date());
                    }
                }
                "STATUS" => {
                    completed = val.trim().eq_ignore_ascii_case("COMPLETED");
                }
                "DUE" => {
                    if let Some((dt, _)) = parse_ics_datetime(val) {
                        due_date = Some(dt.date());
                    }
                }
                _ => {}
            }
        }
    }

    on_progress(total_lines, total_lines);
    Ok((events, todos))
}

fn parse_ics_datetime(raw: &str) -> Option<(NaiveDateTime, bool)> {
    let clean = raw.trim().trim_end_matches('Z');
    if clean.len() == 8 {
        let d = NaiveDate::parse_from_str(clean, "%Y%m%d").ok()?;
        Some((d.and_time(NaiveTime::from_hms_opt(0, 0, 0)?), true))
    } else if clean.len() >= 15 {
        let dt = NaiveDateTime::parse_from_str(&clean[..15], "%Y%m%dT%H%M%S").ok()?;
        Some((dt, false))
    } else {
        None
    }
}

fn escape_ics(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
}

fn unescape_ics(s: &str) -> String {
    s.replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}

/// 导出为 CSV 格式
#[allow(dead_code)]
pub fn export_to_csv(events: &[Event], todos: &[Todo]) -> String {
    let mut csv = String::from("TYPE,TITLE,URGENCY,START_OR_DUE,END,ALL_DAY,PRIORITY,COMPLETED,NOTES\n");
    for ev in events {
        csv.push_str(&format!(
            "EVENT,\"{}\",{},{},{},{},,,\"{}\"\n",
            ev.summary.replace('"', "'"),
            ev.urgency.as_str(),
            ev.start_time.format("%Y-%m-%d %H:%M"),
            ev.end_time.format("%Y-%m-%d %H:%M"),
            ev.is_all_day,
            ev.description.replace('"', "'")
        ));
    }
    for td in todos {
        csv.push_str(&format!(
            "TODO,\"{}\",,{},,,{},{},\"{}\"\n",
            td.title.replace('"', "'"),
            td.due_date.map(|d| d.to_string()).unwrap_or_default(),
            td.priority,
            td.completed,
            td.description.replace('"', "'")
        ));
    }
    csv
}

pub fn export_to_json(calendars: &[Calendar], events: &[Event], todos: &[Todo]) -> Result<String> {
    let bundle = BackupBundle {
        version: "1.1".into(),
        calendars: calendars.to_vec(),
        events: events.to_vec(),
        todos: todos.to_vec(),
    };
    serde_json::to_string_pretty(&bundle).context("JSON 序列化失败")
}

pub fn import_from_json(raw: &str) -> Result<BackupBundle> {
    serde_json::from_str::<BackupBundle>(raw).context("JSON 备份解析失败")
}
