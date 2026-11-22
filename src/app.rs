use crate::db::Database;
use crate::models::{Calendar, Event, ThemeConfig, Todo};
use crate::sync::caldav::{SyncAccountConfig, SyncEngine, SyncMode};
use crate::sync::ical;
use crate::ui::{
    calendar_grid,
    editors::{self, CalendarDraft, EventDraft, TodoDraft},
    parse_hex_color, settings, todo_panel,
};
use chrono::{Duration, Local, NaiveDate};
use eframe::egui::{self, Color32, RichText};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedSyncConfig {
    provider: String,
    server_url: String,
    username: String,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum MainTab {
    Calendar,
    Todos,
    SyncAndBackup,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum CalendarSubView {
    Month,
    Week,
    Day,
    Year,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum TodoSortBy {
    Priority,
    DueDate,
    Calendar,
}

#[derive(Debug, Clone)]
pub struct ImportProgressState {
    pub active: bool,
    pub stage: String,
    pub current: usize,
    pub total: usize,
    pub finished_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SyncProgressState {
    pub active: bool,
    pub stage: String,
    pub finished_message: Option<String>,
}

pub struct RustCalApp {
    pub db: Database,
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
    pub todos: Vec<Todo>,

    pub active_tab: MainTab,
    pub sub_view: CalendarSubView,
    pub selected_date: NaiveDate,

    pub search_query: String,

    pub todo_sort: TodoSortBy,
    pub show_completed_todos: bool,

    pub event_draft: Option<EventDraft>,
    pub todo_draft: Option<TodoDraft>,
    pub calendar_draft: Option<CalendarDraft>,

    pub theme: ThemeConfig,
    pub sync_config: SyncAccountConfig,
    pub sync_mode: SyncMode,
    pub status_message: String,
    pub import_state: Arc<Mutex<ImportProgressState>>,
    pub sync_state: Arc<Mutex<SyncProgressState>>,
}

impl RustCalApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, cjk_font_loaded: bool) -> Self {
        let db = Database::open().expect("无法初始化本地 SQLite 数据库");
        let calendars = db.list_calendars().unwrap_or_default();
        let events = db.list_events().unwrap_or_default();
        let todos = db.list_todos().unwrap_or_default();
        let theme = ThemeConfig::default();
        let sync_config = Self::load_sync_config();

        let init_msg = if cjk_font_loaded {
            "本地离线数据库与中文字体已就绪".into()
        } else {
            "⚠ 未探测到系统默认中文字体，如中文显示方框请检查系统字体目录".into()
        };

        Self {
            db,
            calendars,
            events,
            todos,
            active_tab: MainTab::Calendar,
            sub_view: CalendarSubView::Month,
            selected_date: Local::now().date_naive(),
            search_query: String::new(),
            todo_sort: TodoSortBy::Priority,
            show_completed_todos: true,
            event_draft: None,
            todo_draft: None,
            calendar_draft: None,
            theme,
            sync_mode: SyncMode::Merge,
            sync_config,
            status_message: init_msg,
            import_state: Arc::new(Mutex::new(ImportProgressState {
                active: false,
                stage: String::new(),
                current: 0,
                total: 0,
                finished_message: None,
            })),
            sync_state: Arc::new(Mutex::new(SyncProgressState {
                active: false,
                stage: String::new(),
                finished_message: None,
            })),
        }
    }

    pub fn is_importing(&self) -> bool {
        self.import_state
            .lock()
            .map(|s| s.active)
            .unwrap_or(false)
    }

    pub fn poll_import_progress(&mut self) -> Option<ImportProgressState> {
        let mut need_reload = false;
        let mut snapshot = None;

        if let Ok(mut state) = self.import_state.lock() {
            if let Some(msg) = state.finished_message.take() {
                self.status_message = msg;
                need_reload = true;
            }
            if state.active {
                snapshot = Some(state.clone());
            }
        }

        if need_reload {
            self.reload_all();
        }
        snapshot
    }

    pub fn is_syncing(&self) -> bool {
        self.sync_state.lock().map(|s| s.active).unwrap_or(false)
    }

    /// Run all CalDAV network and database work on a worker thread. The GUI thread
    /// only polls the result and reloads its view after the worker has finished.
    pub fn start_async_caldav_sync(&mut self, ctx: egui::Context) {
        if self.is_syncing() || self.is_importing() {
            return;
        }
        let cfg = self.sync_config.clone();
        let mode = self.sync_mode;
        let calendars = self.calendars.clone();
        let events = self.events.clone();
        let todos = self.todos.clone();
        let state = Arc::clone(&self.sync_state);
        if let Ok(mut s) = state.lock() {
            s.active = true;
            s.stage = format!("正在执行{}，网络请求在后台运行…", mode.label());
            s.finished_message = None;
        }
        self.status_message = format!("CalDAV：正在执行{}…", mode.label());
        ctx.request_repaint();

        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<String> {
                // A dedicated SQLite connection avoids sharing rusqlite::Connection
                // across threads and keeps the egui event loop responsive.
                let mut worker_db = Database::open()?;
                SyncEngine::sync_with_mode(
                    &cfg, &mut worker_db, &calendars, &events, &todos, mode,
                )
            })();
            if let Ok(mut s) = state.lock() {
                s.active = false;
                s.stage.clear();
                s.finished_message = Some(match result {
                    Ok(summary) => format!("✅ 同步成功：{}", summary),
                    Err(e) => format!("❌ 同步失败：{}", e),
                });
            }
            ctx.request_repaint();
        });
    }

    /// Test the server connection in the worker too; even a slow DNS/TLS handshake
    /// must never block the egui event loop.
    pub fn start_async_connection_test(&mut self, ctx: egui::Context) {
        if self.is_syncing() || self.is_importing() {
            return;
        }
        let cfg = self.sync_config.clone();
        let state = Arc::clone(&self.sync_state);
        if let Ok(mut s) = state.lock() {
            s.active = true;
            s.stage = "正在后台测试 CalDAV 连接…".into();
            s.finished_message = None;
        }
        self.status_message = "CalDAV：正在后台测试连接…".into();
        ctx.request_repaint();
        std::thread::spawn(move || {
            let result = SyncEngine::test_connection(&cfg);
            if let Ok(mut s) = state.lock() {
                s.active = false;
                s.stage.clear();
                s.finished_message = Some(match result {
                    Ok(msg) => format!("✅ 连接成功：{}", msg),
                    Err(e) => format!("❌ 连接失败：{}", e),
                });
            }
            ctx.request_repaint();
        });
    }

    /// Consume worker completion on the UI thread and refresh visible records.
    pub fn poll_sync_result(&mut self) {
        let finished = self.sync_state.lock().ok().and_then(|mut s| s.finished_message.take());
        if let Some(message) = finished {
            self.status_message = message;
            self.reload_all();
        }
    }

    pub fn start_async_ics_import(&mut self, path: PathBuf, ctx: egui::Context) {
        if self.is_importing() || self.is_syncing() {
            return;
        }
        let default_cal = self.default_calendar_id();
        let state_arc = Arc::clone(&self.import_state);

        if let Ok(mut st) = state_arc.lock() {
            st.active = true;
            st.stage = "1/3 正在读取文件并识别多语言编码...".into();
            st.current = 0;
            st.total = 100;
            st.finished_message = None;
        }
        ctx.request_repaint();

        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<String> {
                let content = ical::read_text_file_multilingual(&path)?;

                if let Ok(mut st) = state_arc.lock() {
                    st.stage = "2/3 正在解析 VEVENT 日程与 VTODO 任务...".into();
                    st.current = 10;
                    st.total = 100;
                }
                ctx.request_repaint();

                let state_parse = Arc::clone(&state_arc);
                let ctx_parse = ctx.clone();
                let (events, todos) =
                    ical::import_from_ics_with_progress(&content, &default_cal, |cur, tot| {
                        if let Ok(mut st) = state_parse.lock() {
                            st.current = cur;
                            st.total = tot;
                        }
                        ctx_parse.request_repaint();
                    })?;

                let ev_cnt = events.len();
                let td_cnt = todos.len();

                if let Ok(mut st) = state_arc.lock() {
                    st.stage = format!("3/3 正在批量写入数据库 (共 {} 条)...", ev_cnt + td_cnt);
                    st.current = 0;
                    st.total = (ev_cnt + td_cnt).max(1);
                }
                ctx.request_repaint();

                let mut bg_db = Database::open()?;
                let state_db = Arc::clone(&state_arc);
                let ctx_db = ctx.clone();
                bg_db.bulk_import_with_progress(&[], &events, &todos, |cur, tot| {
                    if let Ok(mut st) = state_db.lock() {
                        st.current = cur;
                        st.total = tot.max(1);
                    }
                    ctx_db.request_repaint();
                })?;

                Ok(format!(
                    "✅ 后台导入完成：共导入 {} 条日程与 {} 条任务",
                    ev_cnt, td_cnt
                ))
            })();

            if let Ok(mut st) = state_arc.lock() {
                st.active = false;
                st.finished_message = Some(match result {
                    Ok(msg) => msg,
                    Err(e) => format!("❌ 导入失败: {}", e),
                });
            }
            ctx.request_repaint();
        });
    }

    pub fn start_async_json_import(&mut self, path: PathBuf, ctx: egui::Context) {
        if self.is_importing() || self.is_syncing() {
            return;
        }
        let state_arc = Arc::clone(&self.import_state);

        if let Ok(mut st) = state_arc.lock() {
            st.active = true;
            st.stage = "正在读取并恢复 JSON 全量备份...".into();
            st.current = 0;
            st.total = 100;
            st.finished_message = None;
        }
        ctx.request_repaint();

        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<String> {
                let raw = ical::read_text_file_multilingual(&path)?;
                let bundle = ical::import_from_json(&raw)?;
                let mut bg_db = Database::open()?;
                let state_db = Arc::clone(&state_arc);
                let ctx_db = ctx.clone();
                bg_db.bulk_import_with_progress(
                    &bundle.calendars,
                    &bundle.events,
                    &bundle.todos,
                    |cur, tot| {
                        if let Ok(mut st) = state_db.lock() {
                            st.current = cur;
                            st.total = tot.max(1);
                        }
                        ctx_db.request_repaint();
                    },
                )?;
                Ok(format!(
                    "✅ JSON 备份恢复完成（{} 个日历、{} 条日程、{} 条任务）",
                    bundle.calendars.len(),
                    bundle.events.len(),
                    bundle.todos.len()
                ))
            })();

            if let Ok(mut st) = state_arc.lock() {
                st.active = false;
                st.finished_message = Some(match result {
                    Ok(msg) => msg,
                    Err(e) => format!("❌ 备份恢复失败: {}", e),
                });
            }
            ctx.request_repaint();
        });
    }

    fn load_sync_config() -> SyncAccountConfig {
        let default_cfg = SyncAccountConfig {
            provider: "CalDAV".into(),
            server_url: "https://caldav.example.com/dav/".into(),
            username: String::new(),
            token_or_password: String::new(),
        };
        let path = Database::sync_config_path();
        if let Ok(raw) = std::fs::read_to_string(path) {
            if let Ok(saved) = serde_json::from_str::<PersistedSyncConfig>(&raw) {
                return SyncAccountConfig {
                    provider: if saved.provider.is_empty() {
                        "CalDAV".into()
                    } else {
                        saved.provider
                    },
                    server_url: saved.server_url,
                    username: saved.username,
                    token_or_password: String::new(),
                };
            }
        }
        default_cfg
    }

    pub fn save_sync_config(&self) {
        let path = Database::sync_config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let persisted = PersistedSyncConfig {
            provider: self.sync_config.provider.clone(),
            server_url: self.sync_config.server_url.clone(),
            username: self.sync_config.username.clone(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&persisted) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn clear_sync_config(&mut self) {
        self.sync_config.server_url.clear();
        self.sync_config.username.clear();
        self.sync_config.token_or_password.clear();
        let path = Database::sync_config_path();
        let _ = std::fs::remove_file(path);
        self.status_message = "✅ 已清空保存的同步服务器与账号配置".into();
    }

    fn apply_theme_visuals(&self, ctx: &egui::Context) {
        ctx.set_visuals(egui::Visuals::light());
    }

    pub fn reload_all(&mut self) {
        self.calendars = self.db.list_calendars().unwrap_or_default();
        self.events = self.db.list_events().unwrap_or_default();
        self.todos = self.db.list_todos().unwrap_or_default();
    }

    pub fn default_calendar_id(&self) -> String {
        self.calendars
            .first()
            .map(|c| c.id.clone())
            .unwrap_or_default()
    }

    pub fn calendar_color(&self, cal_id: &str) -> Color32 {
        self.calendars
            .iter()
            .find(|c| c.id == cal_id)
            .map(|c| parse_hex_color(&c.color))
            .unwrap_or(Color32::from_rgb(37, 99, 235))
    }

    pub fn is_calendar_visible(&self, cal_id: &str) -> bool {
        self.calendars
            .iter()
            .find(|c| c.id == cal_id)
            .map(|c| c.is_visible)
            .unwrap_or(true)
    }

    pub fn visible_events_on(&self, date: NaiveDate) -> Vec<Event> {
        let q = self.search_query.trim().to_lowercase();
        self.events
            .iter()
            .filter(|ev| self.is_calendar_visible(&ev.calendar_id))
            .filter(|ev| ev.occurs_on(date))
            .filter(|ev| {
                q.is_empty()
                    || ev.summary.to_lowercase().contains(&q)
                    || ev.description.to_lowercase().contains(&q)
            })
            .cloned()
            .collect()
    }

    pub fn filtered_sorted_todos(&self) -> Vec<Todo> {
        let q = self.search_query.trim().to_lowercase();
        let mut list: Vec<Todo> = self
            .todos
            .iter()
            .filter(|td| self.is_calendar_visible(&td.calendar_id))
            .filter(|td| self.show_completed_todos || !td.completed)
            .filter(|td| {
                q.is_empty()
                    || td.title.to_lowercase().contains(&q)
                    || td.description.to_lowercase().contains(&q)
            })
            .cloned()
            .collect();

        match self.todo_sort {
            TodoSortBy::Priority => list.sort_by_key(|t| (t.completed, t.priority, t.due_date)),
            TodoSortBy::DueDate => list.sort_by_key(|t| (t.completed, t.due_date, t.priority)),
            TodoSortBy::Calendar => list
                .sort_by(|a, b| a.calendar_id.cmp(&b.calendar_id).then(a.priority.cmp(&b.priority))),
        }
        list
    }

    pub fn shift_selected_date(&mut self, dir: i64) {
        let days = match self.sub_view {
            CalendarSubView::Day => 1,
            CalendarSubView::Week => 7,
            CalendarSubView::Month => 30,
            CalendarSubView::Year => 365,
        };
        self.selected_date += Duration::days(dir * days);
    }

    pub fn open_new_event_modal(&mut self, date: NaiveDate) {
        self.event_draft = Some(EventDraft::new_for_date(date, self.default_calendar_id()));
    }

    pub fn open_edit_event_modal(&mut self, ev: Event) {
        self.event_draft = Some(EventDraft::from_event(&ev));
    }

    pub fn open_new_todo_modal(&mut self) {
        self.todo_draft = Some(TodoDraft::new(self.default_calendar_id()));
    }

    pub fn open_edit_todo_modal(&mut self, td: Todo) {
        self.todo_draft = Some(TodoDraft::from_todo(&td));
    }

    pub fn save_event(&mut self, ev: Event) {
        if self.is_syncing() { self.status_message = "同步进行中，请稍后再修改日程".into(); return; }
        let _ = self.db.upsert_event(&ev);
        self.reload_all();
    }

    pub fn remove_event(&mut self, id: &str) {
        if self.is_syncing() { self.status_message = "同步进行中，请稍后再删除日程".into(); return; }
        let _ = self.db.delete_event(id);
        self.reload_all();
    }

    pub fn save_todo(&mut self, td: Todo) {
        if self.is_syncing() { self.status_message = "同步进行中，请稍后再修改任务".into(); return; }
        let _ = self.db.upsert_todo(&td);
        self.reload_all();
    }

    pub fn remove_todo(&mut self, id: &str) {
        if self.is_syncing() { self.status_message = "同步进行中，请稍后再删除任务".into(); return; }
        let _ = self.db.delete_todo(id);
        self.reload_all();
    }

    pub fn save_calendar(&mut self, cal: Calendar) {
        if self.is_syncing() { self.status_message = "同步进行中，请稍后再修改日历".into(); return; }
        let _ = self.db.upsert_calendar(&cal);
        self.reload_all();
    }


}

impl eframe::App for RustCalApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_sync_result();
        if self.is_syncing() {
            ctx.request_repaint_after(StdDuration::from_millis(180));
        }
        self.apply_theme_visuals(ctx);

        let fg_color = parse_hex_color(&self.theme.fg_color);
        let mobile_layout = ctx.screen_rect().width() < 680.0;

        // 手机端将应用标题、主导航和搜索框分成独立行，保证触控目标足够大。
        egui::TopBottomPanel::top("top_nav").show(ctx, |ui| {
            if mobile_layout {
                ui.vertical(|ui| {
                    // 第一行：应用标题独占一行
                    ui.heading(RichText::new("🦀 RustCal").strong().color(fg_color));
                    ui.add_space(2.0);

                    // 第二行：主功能导航
                    ui.horizontal_wrapped(|ui| {
                        ui.selectable_value(&mut self.active_tab, MainTab::Calendar, "📅 日历视图");
                        ui.selectable_value(&mut self.active_tab, MainTab::Todos, "✅ 任务看板");
                        ui.selectable_value(
                            &mut self.active_tab,
                            MainTab::SyncAndBackup,
                            "☁ 同步与备份",
                        );
                    });

                    // 第三行：搜索框单独占行，避免挤压导航按钮
                    ui.horizontal(|ui| {
                        if !self.search_query.is_empty() && ui.small_button("✖ 清除").clicked() {
                            self.search_query.clear();
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.search_query)
                                .id(egui::Id::new("global_search_input"))
                                .hint_text("🔍 搜索日程或任务…")
                                .desired_width(ui.available_width().max(120.0)),
                        );
                    });
                });
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.heading(RichText::new("🦀 RustCal").strong().color(fg_color));
                    ui.separator();
                    ui.selectable_value(&mut self.active_tab, MainTab::Calendar, "📅 日历视图");
                    ui.selectable_value(&mut self.active_tab, MainTab::Todos, "✅ 任务看板");
                    ui.selectable_value(
                        &mut self.active_tab,
                        MainTab::SyncAndBackup,
                        "☁ 同步与备份",
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !self.search_query.is_empty() && ui.small_button("✖").clicked() {
                            self.search_query.clear();
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.search_query)
                                .id(egui::Id::new("global_search_input"))
                                .hint_text("🔍 搜索日程或任务…")
                                .desired_width(220.0),
                        );
                    });
                });
            }
        });

        // 桌面端保留侧栏；手机端把日历列表放进可折叠区域。
        if !mobile_layout {
        egui::SidePanel::left("left_sidebar")
            .default_width(230.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(RichText::new("🗂 我的日历").color(fg_color));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("➕").on_hover_text("新建日历").clicked() {
                            self.calendar_draft = Some(CalendarDraft::default());
                        }
                    });
                });
                ui.separator();

                let mut updated_cal: Option<Calendar> = None;
                let mut delete_cal_id: Option<String> = None;

                for cal in &self.calendars {
                    ui.horizontal(|ui| {
                        let mut vis = cal.is_visible;
                        if ui.checkbox(&mut vis, "").changed() {
                            let mut c = cal.clone();
                            c.is_visible = vis;
                            updated_cal = Some(c);
                        }
                        ui.colored_label(parse_hex_color(&cal.color), "■");
                        ui.label(RichText::new(&cal.name).color(fg_color));

                        if self.calendars.len() > 1 {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("×").clicked() {
                                        delete_cal_id = Some(cal.id.clone());
                                    }
                                },
                            );
                        }
                    });
                }

                if let Some(c) = updated_cal {
                    self.save_calendar(c);
                }
                if let Some(id) = delete_cal_id {
                    let _ = self.db.delete_calendar(&id);
                    self.reload_all();
                }

                ui.add_space(16.0);
                ui.separator();
                ui.heading(RichText::new("⚡ 紧急待办速览").color(fg_color));
                let top_todos: Vec<Todo> = self
                    .filtered_sorted_todos()
                    .into_iter()
                    .filter(|t| !t.completed)
                    .take(5)
                    .collect();
                if top_todos.is_empty() {
                    ui.label(RichText::new("所有任务均已完成 🎉").color(Color32::GRAY));
                } else {
                    for td in top_todos {
                        ui.label(
                            RichText::new(format!("[P{}] {}", td.priority, td.title))
                                .size(12.5)
                                .color(fg_color),
                        );
                    }
                }
            });
        }

        if mobile_layout {
            egui::TopBottomPanel::top("mobile_calendars").show(ctx, |ui| {
                ui.collapsing("🗂 我的日历", |ui| {
                    if ui.button("➕ 添加独立日历").clicked() {
                        self.calendar_draft = Some(CalendarDraft::default());
                    }
                    ui.separator();
                    let mut updated: Option<Calendar> = None;
                    let mut delete_id: Option<String> = None;
                    for calendar in &self.calendars {
                        ui.horizontal(|ui| {
                            let mut visible = calendar.is_visible;
                            if ui.checkbox(&mut visible, "").changed() {
                                let mut changed = calendar.clone();
                                changed.is_visible = visible;
                                updated = Some(changed);
                            }
                            ui.colored_label(parse_hex_color(&calendar.color), "■");
                            ui.label(&calendar.name);
                            if self.calendars.len() > 1 && ui.small_button("×").clicked() {
                                delete_id = Some(calendar.id.clone());
                            }
                        });
                    }
                    if let Some(calendar) = updated { self.save_calendar(calendar); }
                    if let Some(id) = delete_id {
                        let _ = self.db.delete_calendar(&id);
                        self.reload_all();
                    }
                });
            });
        }

        // 底部状态栏
        egui::TopBottomPanel::bottom("bottom_status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&self.status_message).size(12.0).color(fg_color));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "日程: {} | 任务: {}",
                            self.events.len(),
                            self.todos.len()
                        ))
                        .size(12.0)
                        .color(Color32::GRAY),
                    );
                });
            });
        });

        // 中央主工作区
        egui::CentralPanel::default().show(ctx, |ui| match self.active_tab {
            MainTab::Calendar => calendar_grid::render_calendar_workspace(self, ui),
            MainTab::Todos => todo_panel::render_todo_workspace(self, ui),
            MainTab::SyncAndBackup => settings::render_sync_and_settings(self, ui),
        });

        // 渲染模态编辑弹窗与导入进度条
        editors::render_modals(self, ctx);

    }
}
