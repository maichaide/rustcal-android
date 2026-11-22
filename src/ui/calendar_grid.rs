use crate::app::{CalendarSubView, RustCalApp};
use crate::models::{Event, EventUrgency};
use crate::ui::parse_hex_color;
use chrono::{Datelike, Duration, Local, NaiveDate, Timelike};
use eframe::egui::{self, Color32, RichText, Stroke};

pub fn render_calendar_workspace(app: &mut RustCalApp, ui: &mut egui::Ui) {
    let fg_color = parse_hex_color(&app.theme.fg_color);
    let mobile_layout = ui.available_width() < 680.0;

    // 手机端将日期导航和视图切换拆成两行，按钮保持足够的触控面积。
    ui.horizontal_wrapped(|ui| {
        if ui.button("◀ 上一周期").clicked() {
            app.shift_selected_date(-1);
        }
        if ui.button("今天").clicked() {
            app.selected_date = Local::now().date_naive();
        }
        if ui.button("下一周期 ▶").clicked() {
            app.shift_selected_date(1);
        }
        if !mobile_layout {
            ui.separator();
        }
        ui.label(
            RichText::new(format!(
                "{}年{:02}月{:02}日 {}",
                app.selected_date.year(),
                app.selected_date.month(),
                app.selected_date.day(),
                weekday_cn(app.selected_date.weekday())
            ))
            .size(12.5)
            .color(fg_color),
        );
    });

    ui.horizontal_wrapped(|ui| {
        if ui.button("➕ 新建日程").clicked() {
            app.open_new_event_modal(app.selected_date);
        }
        ui.label("视图：");
        egui::ComboBox::from_id_salt("calendar_subview_selector")
            .selected_text(match app.sub_view {
                CalendarSubView::Month => "月视图",
                CalendarSubView::Week => "周视图",
                CalendarSubView::Day => "日视图",
                CalendarSubView::Year => "年视图",
            })
            .width(110.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut app.sub_view, CalendarSubView::Month, "月视图");
                ui.selectable_value(&mut app.sub_view, CalendarSubView::Week, "周视图");
                ui.selectable_value(&mut app.sub_view, CalendarSubView::Day, "日视图");
                ui.selectable_value(&mut app.sub_view, CalendarSubView::Year, "年视图");
            });
    });
    ui.separator();

    if mobile_layout {
        // 手机端：日历固定在上方，详情独立滚动显示在日历下方。
        let total_h = ui.available_height();
        let calendar_h = (total_h * 0.48).clamp(250.0, 360.0).min((total_h - 140.0).max(180.0));
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), calendar_h),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                let available_width = ui.available_width();
                ui.set_max_width(available_width);
                ui.group(|ui| {
                    let group_width = ui.available_width();
                    ui.set_max_width(group_width);
                    egui::ScrollArea::both()
                        .id_salt("mobile_calendar_scroll")
                        .auto_shrink([false, true])
                        .show(ui, |ui| match app.sub_view {
                            CalendarSubView::Month => render_month_grid(app, ui),
                            CalendarSubView::Week => render_week_view(app, ui),
                            CalendarSubView::Day => render_day_timeline(app, ui),
                            CalendarSubView::Year => render_year_view(app, ui),
                        });
                });
            },
        );

        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("mobile_event_details_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let available_width = ui.available_width();
                ui.set_width(available_width);
                ui.group(|ui| {
                    let group_width = ui.available_width();
                    ui.set_width(group_width);
                    if app.sub_view == CalendarSubView::Year {
                        render_year_all_events_panel(app, ui);
                    } else {
                        render_selected_day_inspector(app, ui);
                    }
                });
            });
    } else {
        // 桌面端保留左右分栏布局。
        let total_avail_w = ui.available_width();
        let total_avail_h = ui.available_height();
        let gap = 10.0;
        let left_w = ((total_avail_w - gap) * 0.56).max(280.0);
        let right_w = (total_avail_w - gap - left_w).max(240.0);

        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(left_w, total_avail_h),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.set_max_width(left_w);
                    ui.group(|ui| {
                        ui.set_min_height((total_avail_h - 8.0).max(120.0));
                        ui.set_max_width(left_w - 12.0);
                        egui::ScrollArea::both()
                            .id_salt("left_calendar_scroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| match app.sub_view {
                                CalendarSubView::Month => render_month_grid(app, ui),
                                CalendarSubView::Week => render_week_view(app, ui),
                                CalendarSubView::Day => render_day_timeline(app, ui),
                                CalendarSubView::Year => render_year_view(app, ui),
                            });
                    });
                },
            );

            ui.add_space(gap);
            ui.allocate_ui_with_layout(
                egui::vec2(right_w, total_avail_h),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.set_max_width(right_w);
                    ui.group(|ui| {
                        ui.set_min_height((total_avail_h - 8.0).max(120.0));
                        ui.set_max_width(right_w - 12.0);
                        egui::ScrollArea::vertical()
                            .id_salt("desktop_event_details_scroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                if app.sub_view == CalendarSubView::Year {
                                    render_year_all_events_panel(app, ui);
                                } else {
                                    render_selected_day_inspector(app, ui);
                                }
                            });
                    });
                },
            );
        });
    }
}

fn render_month_grid(app: &mut RustCalApp, ui: &mut egui::Ui) {
    let year = app.selected_date.year();
    let month = app.selected_date.month();
    let first_day = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let start_offset = first_day.weekday().num_days_from_monday() as i64;
    let grid_start = first_day - Duration::days(start_offset);
    let today = Local::now().date_naive();

    let date_fg_color = parse_hex_color(&app.theme.month_date_fg_color);
    let custom_cell_bg = if app.theme.month_cell_transparent {
        Color32::TRANSPARENT
    } else {
        parse_hex_color(&app.theme.month_cell_bg_color)
    };

    // 图例说明条（紧凑换行布局）
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("图例：").size(11.0).color(date_fg_color));
        for urg in EventUrgency::all() {
            let c = parse_hex_color(urg.bar_hex());
            let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 3.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0_f32, c);
            let short_name = match urg {
                EventUrgency::ImportantUrgent => "重要紧急",
                EventUrgency::ImportantNotUrgent => "重要不紧急",
                EventUrgency::NormalNotUrgent => "一般不紧急",
                EventUrgency::NormalUrgent => "一般紧急",
                EventUrgency::ReminderOnly => "仅供提醒",
            };
            ui.label(RichText::new(short_name).size(10.5).color(date_fg_color));
            ui.add_space(2.0);
        }
    });
    ui.add_space(4.0);

    // 精确扣除列间距与边框开销，计算紧凑自适应单元格尺寸
    let spacing_x = 2.0;
    let spacing_y = 2.0;
    let avail_w = ui.available_width().max(245.0);
    let col_w = ((avail_w - spacing_x * 6.0) / 7.0).floor().max(30.0);
    // 高度根据可用空间自适应缩放（在 42px ~ 54px 之间紧凑显示，避免撑爆窗口）
    let avail_h = (ui.available_height() - 22.0).max(240.0);
    let cell_h = ((avail_h - spacing_y * 5.0) / 6.0).clamp(38.0, 44.0);

    let headers = ["一", "二", "三", "四", "五", "六", "日"];
    egui::Grid::new("month_grid_header")
        .num_columns(7)
        .spacing([spacing_x, 2.0])
        .min_col_width(col_w)
        .max_col_width(col_w)
        .show(ui, |ui| {
            for h in headers {
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(format!("周{}", h))
                            .size(10.5)
                            .strong()
                            .color(date_fg_color),
                    );
                });
            }
            ui.end_row();
        });

    ui.separator();

    egui::Grid::new("month_grid_cells")
        .num_columns(7)
        .spacing([spacing_x, spacing_y])
        .min_col_width(col_w)
        .max_col_width(col_w)
        .show(ui, |ui| {
            for row in 0..6 {
                for col in 0..7 {
                    let cell_date = grid_start + Duration::days(row * 7 + col);
                    let is_current_month = cell_date.month() == month;
                    let is_selected = cell_date == app.selected_date;
                    let is_today = cell_date == today;

                    let day_events = app.visible_events_on(cell_date);
                    let has_events = !day_events.is_empty();

                    let border_stroke = if is_selected {
                        Stroke::new(1.8_f32, Color32::from_rgb(37, 99, 235))
                    } else if is_today {
                        Stroke::new(1.4_f32, Color32::from_rgb(220, 38, 38))
                    } else {
                        Stroke::new(0.6_f32, Color32::from_rgba_unmultiplied(148, 163, 184, 85))
                    };

                    let inner_pad = 2.0;
                    let frame = egui::Frame::none()
                        .fill(custom_cell_bg)
                        .rounding(4.0_f32)
                        .inner_margin(inner_pad)
                        .stroke(border_stroke);

                    let cell_resp = frame.show(ui, |ui| {
                        let content_w = (col_w - inner_pad * 2.0 - 2.0).max(20.0);
                        ui.set_min_width(content_w);
                        ui.set_max_width(content_w);
                        ui.set_min_height(cell_h);
                        ui.set_max_height(cell_h);

                        // 颜色使用默认或用户自定义颜色；非本月日期适当降低透明度区分
                        let num_color = if is_current_month {
                            date_fg_color
                        } else {
                            Color32::from_rgba_unmultiplied(
                                date_fg_color.r(),
                                date_fg_color.g(),
                                date_fg_color.b(),
                                105,
                            )
                        };

                        ui.vertical_centered(|ui| {
                            let mut day_text = RichText::new(format!("{}", cell_date.day()))
                                .size(12.0)
                                .color(num_color);

                            // 若当天有日程安排或为今天，日期字体显示为粗体
                            if has_events || is_today {
                                day_text = day_text.strong();
                            }
                            if is_today {
                                day_text = day_text.underline();
                            }

                            if ui
                                .add(egui::Label::new(day_text).sense(egui::Sense::click()))
                                .clicked()
                            {
                                app.selected_date = cell_date;
                            }

                            ui.add_space(1.0);

                            // 仅以对应紧急程度色条标识日程
                            for ev in day_events.iter().take(4) {
                                let bar_color = parse_hex_color(ev.urgency.bar_hex());
                                let bar_width = (content_w - 4.0).max(12.0);
                                let (bar_rect, _) = ui.allocate_exact_size(
                                    egui::vec2(bar_width, 3.0),
                                    egui::Sense::click(),
                                );
                                ui.painter().rect_filled(bar_rect, 1.5_f32, bar_color);
                                ui.add_space(0.5);
                            }
                        });
                    });

                    if cell_resp.response.interact(egui::Sense::click()).clicked() {
                        app.selected_date = cell_date;
                    }
                }
                ui.end_row();
            }
        });
}

fn render_week_view(app: &mut RustCalApp, ui: &mut egui::Ui) {
    let offset = app.selected_date.weekday().num_days_from_monday() as i64;
    let monday = app.selected_date - Duration::days(offset);
    let fg_color = parse_hex_color(&app.theme.fg_color);

    ui.heading(RichText::new("本周日程概览").color(fg_color).size(12.5));
    ui.separator();
    for i in 0..7 {
        let d = monday + Duration::days(i);
        let is_sel = d == app.selected_date;
        ui.horizontal(|ui| {
            let title = format!("{} ({})", d.format("%m-%d"), weekday_cn(d.weekday()));
            if ui
                .selectable_label(is_sel, RichText::new(title).strong().color(fg_color))
                .clicked()
            {
                app.selected_date = d;
            }
        });
        let evs = app.visible_events_on(d);
        if evs.is_empty() {
            ui.label(RichText::new("  无日程").color(Color32::GRAY).size(12.0));
        } else {
            for ev in evs {
                let color = parse_hex_color(ev.urgency.bar_hex());
                let time_tag = if ev.is_all_day {
                    "[全天]".to_string()
                } else {
                    format!(
                        "{}-{}",
                        ev.start_time.format("%H:%M"),
                        ev.end_time.format("%H:%M")
                    )
                };
                ui.label(RichText::new(format!("  ■ {} {}", time_tag, ev.summary)).color(color));
            }
        }
        ui.separator();
    }
}

fn render_day_timeline(app: &mut RustCalApp, ui: &mut egui::Ui) {
    let fg_color = parse_hex_color(&app.theme.fg_color);
    ui.heading(RichText::new(format!("{} 时间轴", app.selected_date)).color(fg_color).size(12.5));
    let evs = app.visible_events_on(app.selected_date);

    let all_day: Vec<&Event> = evs.iter().filter(|e| e.is_all_day).collect();
    if !all_day.is_empty() {
        ui.group(|ui| {
            ui.label(RichText::new("📌 全天事件").strong().color(fg_color));
            for ev in all_day {
                ui.label(
                    RichText::new(format!("■ {}", ev.summary))
                        .color(parse_hex_color(ev.urgency.bar_hex())),
                );
            }
        });
    }

    for hour in 0..24 {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{:02}:00", hour))
                    .monospace()
                    .color(Color32::GRAY),
            );
            ui.separator();
            for ev in &evs {
                if !ev.is_all_day && ev.start_time.hour() == hour {
                    ui.label(
                        RichText::new(format!(
                            "[{}-{}] {}",
                            ev.start_time.format("%H:%M"),
                            ev.end_time.format("%H:%M"),
                            ev.summary
                        ))
                        .color(parse_hex_color(ev.urgency.bar_hex()))
                        .strong(),
                    );
                }
            }
        });
    }
}

fn render_year_view(app: &mut RustCalApp, ui: &mut egui::Ui) {
    let year = app.selected_date.year();
    let fg_color = parse_hex_color(&app.theme.fg_color);
    ui.heading(RichText::new(format!("{} 年月份导航", year)).color(fg_color).size(12.5));
    ui.label(
        RichText::new("点击月份可切换至对应月视图，右侧同步展示全年按月日程汇总。")
            .size(12.0)
            .color(Color32::GRAY),
    );
    ui.separator();

    let spacing = 8.0;
    let col_w = ((ui.available_width() - spacing * 2.0) / 3.0).max(80.0);

    egui::Grid::new("year_grid")
        .num_columns(3)
        .spacing([spacing, spacing])
        .min_col_width(col_w)
        .max_col_width(col_w)
        .show(ui, |ui| {
            for m in 1..=12 {
                let first = NaiveDate::from_ymd_opt(year, m, 1).unwrap();
                let is_cur_m = app.selected_date.month() == m;

                // 统计该月日程总数
                let days_in_m = days_in_month(year, m);
                let mut month_ev_count = 0usize;
                for d in 1..=days_in_m {
                    if let Some(date) = NaiveDate::from_ymd_opt(year, m, d) {
                        month_ev_count += app.visible_events_on(date).len();
                    }
                }

                ui.group(|ui| {
                    ui.set_min_width(col_w - 12.0);
                    if ui
                        .selectable_label(
                            is_cur_m,
                            RichText::new(format!("📅 {} 月", m)).strong().color(fg_color),
                        )
                        .clicked()
                    {
                        app.selected_date = first;
                        app.sub_view = CalendarSubView::Month;
                    }
                    let count_text = if month_ev_count > 0 {
                        RichText::new(format!("共 {} 项日程", month_ev_count))
                            .strong()
                            .size(12.0)
                            .color(Color32::from_rgb(37, 99, 235))
                    } else {
                        RichText::new("暂无日程")
                            .size(12.0)
                            .color(Color32::GRAY)
                    };
                    ui.label(count_text);
                });
                if m % 3 == 0 {
                    ui.end_row();
                }
            }
        });
}

/// 年视图右侧专属面板：按 1~12 月顺序展示全年的每一项日程安排
fn render_year_all_events_panel(app: &mut RustCalApp, ui: &mut egui::Ui) {
    let year = app.selected_date.year();
    let fg_color = parse_hex_color(&app.theme.fg_color);

    ui.heading(RichText::new(format!("📆 {} 年全年日程明细（按月汇总）", year)).color(fg_color));
    ui.separator();

    let mut edit_target: Option<Event> = None;
    let mut delete_id: Option<String> = None;
    let mut jump_to_date: Option<NaiveDate> = None;

    {
            let mut total_year_events = 0usize;

            for m in 1..=12 {
                let days = days_in_month(year, m);
                let mut month_entries: Vec<(NaiveDate, Event)> = Vec::new();
                for d in 1..=days {
                    if let Some(date) = NaiveDate::from_ymd_opt(year, m, d) {
                        for ev in app.visible_events_on(date) {
                            month_entries.push((date, ev));
                        }
                    }
                }

                total_year_events += month_entries.len();

                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("📂 {} 年 {:02} 月", year, m))
                                .strong()
                                .size(14.5)
                                .color(fg_color),
                        );
                        ui.label(
                            RichText::new(format!("({} 项)", month_entries.len()))
                                .size(12.0)
                                .color(if month_entries.is_empty() {
                                    Color32::GRAY
                                } else {
                                    Color32::from_rgb(37, 99, 235)
                                }),
                        );
                    });

                    if month_entries.is_empty() {
                        ui.label(
                            RichText::new("  本月暂无日程安排")
                                .size(12.0)
                                .color(Color32::GRAY),
                        );
                    } else {
                        ui.add_space(2.0);
                        for (date, ev) in month_entries {
                            let urg_color = parse_hex_color(ev.urgency.bar_hex());
                            ui.horizontal_wrapped(|ui| {
                                ui.colored_label(urg_color, "■");

                                let date_btn = ui.add(
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "{:02}-{:02} ({})",
                                            date.month(),
                                            date.day(),
                                            weekday_cn(date.weekday())
                                        ))
                                        .monospace()
                                        .size(12.0)
                                        .color(Color32::from_rgb(37, 99, 235)),
                                    )
                                    .sense(egui::Sense::click()),
                                );
                                if date_btn.on_hover_text("点击跳转到该日").clicked() {
                                    jump_to_date = Some(date);
                                }

                                let time_str = if ev.is_all_day {
                                    "[全天]".to_string()
                                } else {
                                    format!(
                                        "{}-{}",
                                        ev.start_time.format("%H:%M"),
                                        ev.end_time.format("%H:%M")
                                    )
                                };
                                ui.label(
                                    RichText::new(time_str)
                                        .size(12.0)
                                        .color(Color32::GRAY),
                                );

                                ui.label(
                                    RichText::new(&ev.summary)
                                        .strong()
                                        .size(13.0)
                                        .color(fg_color),
                                );

                                if ui.small_button("✏").on_hover_text("编辑日程").clicked() {
                                    edit_target = Some(ev.clone());
                                }
                                if ui.small_button("🗑").on_hover_text("删除日程").clicked() {
                                    delete_id = Some(ev.id.clone());
                                }
                            });
                        }
                    }
                });
                ui.add_space(4.0);
            }

            if total_year_events == 0 {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("{} 年全年暂无任何日程记录。", year))
                        .color(Color32::GRAY),
                );
            }
    }

    if let Some(d) = jump_to_date {
        app.selected_date = d;
        app.sub_view = CalendarSubView::Day;
    }
    if let Some(ev) = edit_target {
        app.open_edit_event_modal(ev);
    }
    if let Some(id) = delete_id {
        app.remove_event(&id);
    }
}

fn render_selected_day_inspector(app: &mut RustCalApp, ui: &mut egui::Ui) {
    let fg_color = parse_hex_color(&app.theme.fg_color);
    ui.horizontal(|ui| {
        ui.heading(
            RichText::new(format!("📋 {} 日程详情与冲突检查", app.selected_date)).color(fg_color).size(12.5),
        );
    });
    ui.separator();

    let day_events = app.visible_events_on(app.selected_date);

    // 冲突检测提示
    let mut conflicts = Vec::new();
    for i in 0..day_events.len() {
        for j in (i + 1)..day_events.len() {
            if day_events[i].conflicts_with(&day_events[j], app.selected_date) {
                conflicts.push((day_events[i].summary.clone(), day_events[j].summary.clone()));
            }
        }
    }

    if !conflicts.is_empty() {
        egui::Frame::none()
            .fill(Color32::from_rgb(185, 28, 28))
            .rounding(4.0_f32)
            .inner_margin(6.0_f32)
            .show(ui, |ui| {
                ui.label(RichText::new("⚠ 检测到时间冲突：").strong().color(Color32::WHITE));
                for (a, b) in conflicts {
                    ui.label(
                        RichText::new(format!("  • 「{}」 与 「{}」 时间重叠", a, b))
                            .color(Color32::from_rgb(254, 226, 226)),
                    );
                }
            });
        ui.add_space(6.0);
    }

    let mut edit_target: Option<Event> = None;
    let mut delete_id: Option<String> = None;
    let mut skip_once_ev: Option<Event> = None;

    {
            if day_events.is_empty() {
                ui.label(RichText::new("当日暂无预约日程，可点击上方“新建日程”添加。").color(Color32::GRAY));
            }

            for ev in &day_events {
                let urg_color = parse_hex_color(ev.urgency.bar_hex());
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(urg_color, "■");
                        ui.label(RichText::new(&ev.summary).strong().size(15.0).color(fg_color));
                        ui.label(
                            RichText::new(format!("[{}]", ev.urgency.label()))
                                .size(11.5)
                                .color(urg_color),
                        );
                        if ev.recurrence != crate::models::Recurrence::None {
                            ui.label(
                                RichText::new(format!("[重复: {}]", ev.recurrence.label()))
                                    .size(11.0)
                                    .color(Color32::from_rgb(37, 99, 235)),
                            );
                        }
                    });

                    let time_desc = if ev.is_all_day {
                        "全天事件".to_string()
                    } else {
                        format!(
                            "{} ~ {}",
                            ev.start_time.format("%H:%M"),
                            ev.end_time.format("%H:%M")
                        )
                    };
                    ui.label(RichText::new(format!("🕒 {}", time_desc)).color(fg_color));

                    if let Some(rem) = ev.reminder_minutes {
                        ui.label(
                            RichText::new(format!("🔔 提前 {} 分钟提醒", rem))
                                .size(12.0)
                                .color(Color32::from_rgb(180, 83, 9)),
                        );
                    }

                    if !ev.description.is_empty()
                        && !(ev.description.contains("月视图中将以") && ev.description.contains("横条展示"))
                    {
                        ui.label(RichText::new(&ev.description).size(12.5).color(fg_color));
                    }

                    ui.horizontal(|ui| {
                        if ui.small_button("✏ 编辑").clicked() {
                            edit_target = Some(ev.clone());
                        }
                        if ev.recurrence != crate::models::Recurrence::None {
                            if ui.small_button("⏭ 跳过本日(例外)").clicked() {
                                skip_once_ev = Some(ev.clone());
                            }
                        }
                        if ui.small_button("🗑 删除").clicked() {
                            delete_id = Some(ev.id.clone());
                        }
                    });
                });
            }
    }

    if let Some(ev) = edit_target {
        app.open_edit_event_modal(ev);
    }
    if let Some(mut ev) = skip_once_ev {
        ev.exception_dates.push(app.selected_date);
        app.save_event(ev);
    }
    if let Some(id) = delete_id {
        app.remove_event(&id);
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let next_month = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap()
    };
    (next_month - Duration::days(1)).day()
}

fn weekday_cn(w: chrono::Weekday) -> &'static str {
    match w {
        chrono::Weekday::Mon => "周一",
        chrono::Weekday::Tue => "周二",
        chrono::Weekday::Wed => "周三",
        chrono::Weekday::Thu => "周四",
        chrono::Weekday::Fri => "周五",
        chrono::Weekday::Sat => "周六",
        chrono::Weekday::Sun => "周日",
    }
}
