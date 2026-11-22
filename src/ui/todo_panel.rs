use crate::app::{RustCalApp, TodoSortBy};
use crate::models::Todo;
use crate::ui::parse_hex_color;
use chrono::Local;
use eframe::egui::{self, Color32, RichText};

pub fn render_todo_workspace(app: &mut RustCalApp, ui: &mut egui::Ui) {
    let fg_color = parse_hex_color(&app.theme.fg_color);

    ui.horizontal(|ui| {
        ui.heading(RichText::new("✅ 任务与待办管理").color(fg_color));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            egui::ComboBox::from_id_salt("todo_sort_dropdown")
                .selected_text(match app.todo_sort {
                    TodoSortBy::Priority => "按优先级",
                    TodoSortBy::DueDate => "按截止日期",
                    TodoSortBy::Calendar => "按日历分类",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.todo_sort, TodoSortBy::Priority, "按优先级");
                    ui.selectable_value(&mut app.todo_sort, TodoSortBy::DueDate, "按截止日期");
                    ui.selectable_value(&mut app.todo_sort, TodoSortBy::Calendar, "按日历分类");
                });
        });
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut app.show_completed_todos, "显示已完成");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("➕ 新建任务").clicked() {
                app.open_new_todo_modal();
            }
        });
    });

    ui.separator();

    let today = Local::now().date_naive();
    let mut items = app.filtered_sorted_todos();

    let mut toggle_todo: Option<Todo> = None;
    let mut edit_todo: Option<Todo> = None;
    let mut delete_todo_id: Option<String> = None;

    egui::ScrollArea::vertical().show(ui, |ui| {
        if items.is_empty() {
            ui.label(RichText::new("当前筛选条件下暂无任务。").color(Color32::GRAY));
        }

        for td in items.drain(..) {
            let cal_color = app.calendar_color(&td.calendar_id);
            let is_overdue = !td.completed && td.due_date.map(|d| d < today).unwrap_or(false);

            ui.group(|ui| {
                ui.horizontal(|ui| {
                    let mut checked = td.completed;
                    if ui.checkbox(&mut checked, "").changed() {
                        let mut updated = td.clone();
                        updated.completed = checked;
                        toggle_todo = Some(updated);
                    }

                    let prio_color = match td.priority {
                        1..=2 => Color32::from_rgb(220, 38, 38),
                        3..=5 => Color32::from_rgb(217, 119, 6),
                        _ => Color32::from_rgb(100, 116, 139),
                    };
                    ui.label(
                        RichText::new(format!("[P{}]", td.priority))
                            .strong()
                            .color(prio_color),
                    );

                    ui.colored_label(cal_color, "●");

                    let title_text = if td.completed {
                        RichText::new(&td.title).strikethrough().color(Color32::GRAY)
                    } else {
                        RichText::new(&td.title).strong().size(15.0).color(fg_color)
                    };
                    ui.label(title_text);

                    if let Some(due) = td.due_date {
                        let due_str = if is_overdue {
                            format!("⚠ 已逾期 ({})", due)
                        } else {
                            format!("📅 截止: {}", due)
                        };
                        let due_color = if is_overdue {
                            Color32::from_rgb(220, 38, 38)
                        } else {
                            Color32::from_rgb(37, 99, 235)
                        };
                        ui.label(RichText::new(due_str).size(12.0).color(due_color));
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("🗑 删除").clicked() {
                            delete_todo_id = Some(td.id.clone());
                        }
                        if ui.small_button("✏ 编辑").clicked() {
                            edit_todo = Some(td.clone());
                        }
                    });
                });

                if !td.description.is_empty() {
                    ui.label(
                        RichText::new(format!("   📝 {}", td.description))
                            .size(12.5)
                            .color(fg_color),
                    );
                }
            });
        }
    });

    if let Some(td) = toggle_todo {
        app.save_todo(td);
    }
    if let Some(td) = edit_todo {
        app.open_edit_todo_modal(td);
    }
    if let Some(id) = delete_todo_id {
        app.remove_todo(&id);
    }
}
