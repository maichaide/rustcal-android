use crate::app::RustCalApp;
use crate::models::{Calendar, Event, EventUrgency, Recurrence, Todo};
use chrono::{NaiveDate, NaiveTime};
use eframe::egui::{self, Color32, RichText};
use uuid::Uuid;

pub struct EventDraft {
    pub id: Option<String>,
    pub calendar_id: String,
    pub summary: String,
    pub description: String,
    pub date_str: String,
    pub start_time_str: String,
    pub end_time_str: String,
    pub is_all_day: bool,
    pub recurrence: Recurrence,
    pub exception_dates: Vec<NaiveDate>,
    pub reminder_str: String,
    pub urgency: EventUrgency,
}

impl EventDraft {
    pub fn new_for_date(date: NaiveDate, default_cal_id: String) -> Self {
        Self {
            id: None,
            calendar_id: default_cal_id,
            summary: String::new(),
            description: String::new(),
            date_str: date.format("%Y-%m-%d").to_string(),
            start_time_str: "09:00".to_string(),
            end_time_str: "10:00".to_string(),
            is_all_day: false,
            recurrence: Recurrence::None,
            exception_dates: vec![],
            reminder_str: "15".to_string(),
            urgency: EventUrgency::NormalNotUrgent,
        }
    }

    pub fn from_event(ev: &Event) -> Self {
        Self {
            id: Some(ev.id.clone()),
            calendar_id: ev.calendar_id.clone(),
            summary: ev.summary.clone(),
            description: ev.description.clone(),
            date_str: ev.start_time.date().format("%Y-%m-%d").to_string(),
            start_time_str: ev.start_time.format("%H:%M").to_string(),
            end_time_str: ev.end_time.format("%H:%M").to_string(),
            is_all_day: ev.is_all_day,
            recurrence: ev.recurrence,
            exception_dates: ev.exception_dates.clone(),
            reminder_str: ev.reminder_minutes.map(|m| m.to_string()).unwrap_or_default(),
            urgency: ev.urgency,
        }
    }
}

pub struct TodoDraft {
    pub id: Option<String>,
    pub calendar_id: String,
    pub title: String,
    pub description: String,
    pub priority: u8,
    pub completed: bool,
    pub due_date_str: String,
}

impl TodoDraft {
    pub fn new(default_cal_id: String) -> Self {
        Self {
            id: None,
            calendar_id: default_cal_id,
            title: String::new(),
            description: String::new(),
            priority: 3,
            completed: false,
            due_date_str: String::new(),
        }
    }

    pub fn from_todo(td: &Todo) -> Self {
        Self {
            id: Some(td.id.clone()),
            calendar_id: td.calendar_id.clone(),
            title: td.title.clone(),
            description: td.description.clone(),
            priority: td.priority,
            completed: td.completed,
            due_date_str: td.due_date.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default(),
        }
    }
}

pub struct CalendarDraft {
    pub name: String,
    pub color: String,
    pub account_type: String,
    pub sync_url: String,
}

impl Default for CalendarDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            color: "#8B5CF6".to_string(),
            account_type: "local".to_string(),
            sync_url: String::new(),
        }
    }
}

pub fn render_modals(app: &mut RustCalApp, ctx: &egui::Context) {
    // 0. 后台导入进度条模态框（防止数千条日程导入时用户误以为假死）
    if let Some(progress) = app.poll_import_progress() {
        egui::Window::new("⏳ 正在导入日程数据...")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                ui.label(RichText::new(&progress.stage).strong().size(14.5));
                ui.add_space(6.0);
                let ratio = if progress.total > 0 {
                    (progress.current as f32 / progress.total as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                ui.add(
                    egui::ProgressBar::new(ratio)
                        .show_percentage()
                        .animate(true),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "进度: {} / {} （后台异步执行中，界面保持响应）",
                        progress.current, progress.total
                    ))
                    .size(12.0)
                    .color(Color32::GRAY),
                );
            });
    }

    // 1. 日程编辑器弹窗
    let mut close_event = false;
    let mut save_ev: Option<Event> = None;
    let calendars = app.calendars.clone();

    if let Some(draft) = &mut app.event_draft {
        egui::Window::new(if draft.id.is_some() {
            "✏ 编辑日程 (Appointment)"
        } else {
            "➕ 新建日程 (Appointment)"
        })
        .collapsible(false)
        .resizable(true)
        .show(ctx, |ui| {
            egui::Grid::new("event_form_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("所属日历:");
                    egui::ComboBox::from_id_salt("ev_cal_combo")
                        .selected_text(
                            calendars
                                .iter()
                                .find(|c| c.id == draft.calendar_id)
                                .map(|c| c.name.as_str())
                                .unwrap_or("默认日历"),
                        )
                        .show_ui(ui, |ui| {
                            for c in &calendars {
                                ui.selectable_value(&mut draft.calendar_id, c.id.clone(), &c.name);
                            }
                        });
                    ui.end_row();

                    ui.label("重要/紧急色标:");
                    egui::ComboBox::from_id_salt("ev_urg_combo")
                        .selected_text(draft.urgency.label())
                        .show_ui(ui, |ui| {
                            for u in EventUrgency::all() {
                                ui.selectable_value(&mut draft.urgency, u, u.label());
                            }
                        });
                    ui.end_row();

                    ui.label("日程标题:");
                    ui.text_edit_singleline(&mut draft.summary);
                    ui.end_row();

                    ui.label("日期 (YYYY-MM-DD):");
                    ui.text_edit_singleline(&mut draft.date_str);
                    ui.end_row();

                    ui.label("全天事件:");
                    ui.checkbox(&mut draft.is_all_day, "全天日程");
                    ui.end_row();

                    if !draft.is_all_day {
                        ui.label("开始时间 (HH:MM):");
                        ui.text_edit_singleline(&mut draft.start_time_str);
                        ui.end_row();

                        ui.label("结束时间 (HH:MM):");
                        ui.text_edit_singleline(&mut draft.end_time_str);
                        ui.end_row();
                    }

                    ui.label("重复周期:");
                    egui::ComboBox::from_id_salt("ev_rec_combo")
                        .selected_text(draft.recurrence.label())
                        .show_ui(ui, |ui| {
                            for r in [
                                Recurrence::None,
                                Recurrence::Daily,
                                Recurrence::Weekly,
                                Recurrence::Monthly,
                                Recurrence::Yearly,
                            ] {
                                ui.selectable_value(&mut draft.recurrence, r, r.label());
                            }
                        });
                    ui.end_row();

                    ui.label("提前提醒 (分钟):");
                    ui.text_edit_singleline(&mut draft.reminder_str);
                    ui.end_row();

                    ui.label("备注 (Markdown):");
                    ui.text_edit_multiline(&mut draft.description);
                    ui.end_row();
                });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("💾 保存").clicked() {
                    let date = NaiveDate::parse_from_str(draft.date_str.trim(), "%Y-%m-%d")
                        .unwrap_or_else(|_| chrono::Local::now().date_naive());
                    let st = NaiveTime::parse_from_str(draft.start_time_str.trim(), "%H:%M")
                        .unwrap_or_else(|_| NaiveTime::from_hms_opt(9, 0, 0).unwrap());
                    let et = NaiveTime::parse_from_str(draft.end_time_str.trim(), "%H:%M")
                        .unwrap_or_else(|_| NaiveTime::from_hms_opt(10, 0, 0).unwrap());

                    save_ev = Some(Event {
                        id: draft.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string()),
                        calendar_id: draft.calendar_id.clone(),
                        summary: if draft.summary.trim().is_empty() {
                            "未命名日程".into()
                        } else {
                            draft.summary.trim().to_string()
                        },
                        description: draft.description.clone(),
                        start_time: date.and_time(st),
                        end_time: date.and_time(et),
                        is_all_day: draft.is_all_day,
                        recurrence: draft.recurrence,
                        exception_dates: draft.exception_dates.clone(),
                        reminder_minutes: draft.reminder_str.trim().parse::<i64>().ok(),
                        urgency: draft.urgency,
                    });
                    close_event = true;
                }
                if ui.button("取消").clicked() {
                    close_event = true;
                }
            });
        });
    }
    if let Some(ev) = save_ev {
        app.save_event(ev);
    }
    if close_event {
        app.event_draft = None;
    }

    // 2. Todo 任务编辑器弹窗
    let mut close_todo = false;
    let mut save_td: Option<Todo> = None;

    if let Some(draft) = &mut app.todo_draft {
        egui::Window::new(if draft.id.is_some() {
            "✏ 编辑任务 (Todo)"
        } else {
            "➕ 新建任务 (Todo)"
        })
        .collapsible(false)
        .show(ctx, |ui| {
            egui::Grid::new("todo_form_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("所属日历:");
                    egui::ComboBox::from_id_salt("td_cal_combo")
                        .selected_text(
                            calendars
                                .iter()
                                .find(|c| c.id == draft.calendar_id)
                                .map(|c| c.name.as_str())
                                .unwrap_or("默认日历"),
                        )
                        .show_ui(ui, |ui| {
                            for c in &calendars {
                                ui.selectable_value(&mut draft.calendar_id, c.id.clone(), &c.name);
                            }
                        });
                    ui.end_row();

                    ui.label("任务标题:");
                    ui.text_edit_singleline(&mut draft.title);
                    ui.end_row();

                    ui.label("优先级 (1最高-9最低):");
                    ui.add(egui::Slider::new(&mut draft.priority, 1..=9));
                    ui.end_row();

                    ui.label("截止日期 (YYYY-MM-DD):");
                    ui.text_edit_singleline(&mut draft.due_date_str);
                    ui.end_row();

                    ui.label("完成状态:");
                    ui.checkbox(&mut draft.completed, "已完成");
                    ui.end_row();

                    ui.label("任务备注:");
                    ui.text_edit_multiline(&mut draft.description);
                    ui.end_row();
                });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("💾 保存").clicked() {
                    let due = NaiveDate::parse_from_str(draft.due_date_str.trim(), "%Y-%m-%d").ok();
                    save_td = Some(Todo {
                        id: draft.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string()),
                        calendar_id: draft.calendar_id.clone(),
                        title: if draft.title.trim().is_empty() {
                            "未命名任务".into()
                        } else {
                            draft.title.trim().to_string()
                        },
                        description: draft.description.clone(),
                        priority: draft.priority,
                        completed: draft.completed,
                        due_date: due,
                    });
                    close_todo = true;
                }
                if ui.button("取消").clicked() {
                    close_todo = true;
                }
            });
        });
    }
    if let Some(td) = save_td {
        app.save_todo(td);
    }
    if close_todo {
        app.todo_draft = None;
    }

    // 3. 新建日历弹窗
    let mut close_cal = false;
    let mut save_cal: Option<Calendar> = None;
    if let Some(draft) = &mut app.calendar_draft {
        egui::Window::new("➕ 添加独立日历")
            .collapsible(false)
            .show(ctx, |ui| {
                egui::Grid::new("cal_form_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("日历名称:");
                        ui.text_edit_singleline(&mut draft.name);
                        ui.end_row();

                        ui.label("颜色 (Hex):");
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(&mut draft.color);
                            for preset in ["#2563EB", "#10B981", "#F59E0B", "#EF4444", "#8B5CF6"] {
                                if ui.small_button(preset).clicked() {
                                    draft.color = preset.to_string();
                                }
                            }
                        });
                        ui.end_row();

                        ui.label("账户类型:");
                        egui::ComboBox::from_id_salt("cal_acc_type")
                            .selected_text(&draft.account_type)
                            .show_ui(ui, |ui| {
                                for t in ["local", "caldav", "google", "outlook"] {
                                    ui.selectable_value(&mut draft.account_type, t.into(), t);
                                }
                            });
                        ui.end_row();

                        ui.label("同步 URL (可选):");
                        ui.text_edit_singleline(&mut draft.sync_url);
                        ui.end_row();
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("创建").clicked() {
                        save_cal = Some(Calendar {
                            id: Uuid::new_v4().to_string(),
                            name: if draft.name.trim().is_empty() {
                                "新日历".into()
                            } else {
                                draft.name.trim().to_string()
                            },
                            color: draft.color.clone(),
                            account_type: draft.account_type.clone(),
                            is_visible: true,
                            sync_url: if draft.sync_url.trim().is_empty() {
                                None
                            } else {
                                Some(draft.sync_url.trim().to_string())
                            },
                        });
                        close_cal = true;
                    }
                    if ui.button("取消").clicked() {
                        close_cal = true;
                    }
                });
            });
    }
    if let Some(cal) = save_cal {
        app.save_calendar(cal);
    }
    if close_cal {
        app.calendar_draft = None;
    }
}
