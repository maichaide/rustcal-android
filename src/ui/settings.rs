use crate::app::RustCalApp;
use crate::db::Database;
use crate::sync::caldav::SyncMode;
use crate::sync::ical;
use crate::ui::parse_hex_color;
use eframe::egui::{self, Color32, RichText};

pub fn render_sync_and_settings(app: &mut RustCalApp, ui: &mut egui::Ui) {
    let fg_color = parse_hex_color(&app.theme.fg_color);
    let is_importing = app.is_importing();

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.heading(RichText::new("🔄 数据导入 / 导出与全量备份 ").color(fg_color));
        ui.label(
            RichText::new(format!(
                "本地 SQLite 数据库路径: {}",
                Database::default_db_path().display()
            ))
            .color(fg_color),
        );
        ui.add_space(6.0);

        #[cfg(not(target_os = "android"))]
        {
        ui.horizontal(|ui| {
            let import_btn = ui.add_enabled(
                !is_importing,
                egui::Button::new("📥 导入 .ics "),
            );
            if import_btn.clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("iCalendar", &["ics"])
                    .pick_file()
                {
                    app.start_async_ics_import(path, ui.ctx().clone());
                }
            }

            if ui.button("📤 导出为 .ics (UTF-8 标准)").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name("rustcal_export.ics")
                    .save_file()
                {
                    let content = ical::export_to_ics(&app.events, &app.todos);
                    // 【关键修复】：显式以 UTF-8 字节流写入文件，避免受 Windows 本地 ANSI/GBK 编码影响
                    match std::fs::write(&path, content.as_bytes()) {
                        Ok(_) => {
                            app.status_message =
                                format!("✅ 已成功导出 UTF-8 ICS 至 {}", path.display())
                        }
                        Err(e) => app.status_message = format!("❌ 导出失败: {}", e),
                    }
                }
            }
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("💾 导出完整 JSON 备份").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name("rustcal_backup.json")
                    .save_file()
                {
                    match ical::export_to_json(&app.calendars, &app.events, &app.todos) {
                        Ok(json) => match std::fs::write(&path, json) {
                            Ok(_) => {
                                app.status_message =
                                    format!("✅ JSON 完整备份已保存至 {}", path.display())
                            }
                            Err(e) => app.status_message = format!("❌ 保存失败: {}", e),
                        },
                        Err(e) => app.status_message = format!("❌ 序列化失败: {}", e),
                    }
                }
            }

            let restore_btn = ui.add_enabled(
                !is_importing,
                egui::Button::new("♻ 从 JSON 备份恢复"),
            );
            if restore_btn.clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("JSON", &["json"])
                    .pick_file()
                {
                    app.start_async_json_import(path, ui.ctx().clone());
                }
            }
        });

        }

        #[cfg(target_os = "android")]
        {
            let import_ics = crate::platform::import_dir().join("rustcal_import.ics");
            let import_json = crate::platform::import_dir().join("rustcal_backup.json");
            let export_ics = crate::platform::export_dir().join("rustcal_export.ics");
            let export_json = crate::platform::export_dir().join("rustcal_backup.json");

            ui.label("Android 应用数据目录（系统沙盒内，无需授予整个手机存储权限）：");
            ui.label(RichText::new("数据库与配置默认保存在应用私有 files 目录；无需额外存储授权。卸载应用或清除应用数据可能会删除这些本地数据，请定期导出备份。")
                .small().color(Color32::GRAY));
            ui.label(RichText::new(format!("导入目录：{}", crate::platform::import_dir().display())).small());
            ui.label(RichText::new(format!("导出目录：{}", crate::platform::export_dir().display())).small());
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!is_importing, egui::Button::new("📥 导入 ICS 文件")).clicked() {
                    if import_ics.exists() {
                        app.start_async_ics_import(import_ics.clone(), ui.ctx().clone());
                    } else {
                        app.status_message = format!("请先将 ICS 文件放入：{}", import_ics.display());
                    }
                }
                if ui.button("📤 导出 ICS 文件").clicked() {
                    let content = ical::export_to_ics(&app.events, &app.todos);
                    app.status_message = match std::fs::write(&export_ics, content.as_bytes()) {
                        Ok(()) => format!("ICS 已导出：{}", export_ics.display()),
                        Err(e) => format!("ICS 导出失败：{e}"),
                    };
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("💾 导出完整 JSON 备份").clicked() {
                    app.status_message = match ical::export_to_json(&app.calendars, &app.events, &app.todos) {
                        Ok(json) => match std::fs::write(&export_json, json) {
                            Ok(()) => format!("JSON 备份已导出：{}", export_json.display()),
                            Err(e) => format!("JSON 备份保存失败：{e}"),
                        },
                        Err(e) => format!("JSON 序列化失败：{e}"),
                    };
                }
                if ui.add_enabled(!is_importing, egui::Button::new("♻ 从 JSON 备份恢复")).clicked() {
                    if import_json.exists() {
                        app.start_async_json_import(import_json.clone(), ui.ctx().clone());
                    } else {
                        app.status_message = format!("请先将 JSON 备份放入：{}", import_json.display());
                    }
                }
            });
            ui.label(RichText::new("提示：当前 Android 版本通过应用私有目录读写文件；可用系统文件管理器的应用数据访问能力或 adb push/pull 交换文件。系统“打开/创建文档”选择器尚未接入。")
                .small().color(Color32::GRAY));
        }

        ui.separator();
        ui.heading(RichText::new("☁ 云端日历同步与协作（CalDAV和WebDAV）").color(fg_color));

        let mut sync_field_changed = false;
        egui::Grid::new("sync_account_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(RichText::new("同步协议 / 生态:").color(fg_color));
                egui::ComboBox::from_id_salt("sync_provider_combo")
                    .selected_text(&app.sync_config.provider)
                    .show_ui(ui, |ui| {
                        for p in ["CalDAV", "WebDAV"] {
                            if ui
                                .selectable_value(&mut app.sync_config.provider, p.into(), p)
                                .changed()
                            {
                                sync_field_changed = true;
                            }
                        }
                    });
                ui.end_row();

                ui.label(RichText::new("服务器 / Endpoint URL:").color(fg_color));
                if ui.text_edit_singleline(&mut app.sync_config.server_url).changed() {
                    sync_field_changed = true;
                }
                ui.end_row();

                ui.label(RichText::new("账号 / Client ID:").color(fg_color));
                if ui.text_edit_singleline(&mut app.sync_config.username).changed() {
                    sync_field_changed = true;
                }
                ui.end_row();

                ui.label(RichText::new("密码 / OAuth Token:").color(fg_color));
                ui.add(
                    egui::TextEdit::singleline(&mut app.sync_config.token_or_password)
                        .password(true),
                );
                ui.end_row();
            });
        if sync_field_changed {
            app.save_sync_config();
        }

        ui.label(RichText::new("同步模式（点击选择）:").strong().color(fg_color));
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut app.sync_mode, SyncMode::Merge, "⇄ 双向合并（默认）");
            ui.selectable_value(&mut app.sync_mode, SyncMode::LocalToCloud, "↑ 本机 → 云端");
            ui.selectable_value(&mut app.sync_mode, SyncMode::CloudToLocal, "↓ 云端 → 本机");
        });
        ui.label(
            RichText::new(
                "首次同步仅合并/创建，不推断删除；首次基线后，才新增、修改和删除。单向模式以所选来源为准。并发修改会保留为独立冲突副本。",
            )
            .small()
            .color(fg_color),
        );
        ui.horizontal(|ui| {
            let busy = app.is_syncing() || app.is_importing();
            let test_button = ui.add_enabled(!busy, egui::Button::new("🔌 后台测试连接"));
            if test_button.clicked() {
                app.start_async_connection_test(ui.ctx().clone());
            }
            let sync_button = ui.add_enabled(!busy, egui::Button::new("🔄 立即同步（后台执行）"));
            if sync_button.clicked() {
                app.start_async_caldav_sync(ui.ctx().clone());
            }
            if ui.button("🗑 清空同步配置").clicked() {
                app.clear_sync_config();
            }
        });
        if let Ok(state) = app.sync_state.lock() {
            if state.active {
                ui.horizontal(|ui| {
                    ui.spinner();
                    let message = if state.stage.is_empty() {
                        "后台任务正在运行，界面仍可正常操作…".to_string()
                    } else {
                        state.stage.clone()
                    };
                    ui.label(message);
                });
            }
        }

        ui.separator();
        #[cfg(has_donate_qr)]
        ui.horizontal(|ui| {
            ui.label(RichText::new("支持 RustCal 项目：").color(fg_color));
            let bytes: &'static [u8] =
                include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/donate_qr.png"));
            ui.add(
                egui::Image::from_bytes("bytes://donate_qr.png", bytes)
                    .fit_to_exact_size(egui::vec2(112.0, 112.0))
                    .maintain_aspect_ratio(false),
            );
        });

        if !app.status_message.is_empty() {
            ui.add_space(10.0);
            let msg_color = if app.status_message.starts_with('❌') {
                Color32::from_rgb(220, 38, 38)
            } else {
                Color32::from_rgb(5, 150, 105)
            };
            ui.label(
                RichText::new(&app.status_message)
                    .strong()
                    .color(msg_color),
            );
        }
    });
}
