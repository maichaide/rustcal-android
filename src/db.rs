use crate::models::{Calendar, Event, EventUrgency, Recurrence, Todo};
use anyhow::Result;
use chrono::{NaiveDate, NaiveDateTime};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::PathBuf;
use uuid::Uuid;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open() -> Result<Self> {
        crate::platform::ensure_user_dirs()?;
        let db_path = Self::default_db_path();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        // 开启 WAL 模式与 NORMAL 同步以大幅提升批量写入性能
        let _ = conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;");
        let db = Self { conn };
        db.init_schema()?;
        db.seed_defaults_if_empty()?;
        Ok(db)
    }

    pub fn default_db_path() -> PathBuf {
        crate::platform::data_dir().join("rustcal.db")
    }

    pub fn theme_config_path() -> PathBuf {
        crate::platform::config_dir().join("theme.json")
    }

    pub fn sync_config_path() -> PathBuf {
        crate::platform::config_dir().join("sync_config.json")
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS calendars (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                color TEXT NOT NULL,
                account_type TEXT NOT NULL,
                is_visible INTEGER NOT NULL DEFAULT 1,
                sync_url TEXT
            );

            CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                calendar_id TEXT NOT NULL,
                summary TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                start_time TEXT NOT NULL,
                end_time TEXT NOT NULL,
                is_all_day INTEGER NOT NULL DEFAULT 0,
                recurrence TEXT NOT NULL DEFAULT 'NONE',
                exception_dates TEXT NOT NULL DEFAULT '',
                reminder_minutes INTEGER,
                urgency TEXT NOT NULL DEFAULT 'NORMAL_NOT_URGENT',
                FOREIGN KEY (calendar_id) REFERENCES calendars(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS todos (
                id TEXT PRIMARY KEY,
                calendar_id TEXT NOT NULL,
                title TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                priority INTEGER NOT NULL DEFAULT 3,
                completed INTEGER NOT NULL DEFAULT 0,
                due_date TEXT,
                FOREIGN KEY (calendar_id) REFERENCES calendars(id) ON DELETE CASCADE
            );

            -- One durable sync baseline per configured CalDAV collection. The baseline
            -- is what lets us distinguish an intentional deletion from an item that
            -- has never been synchronized.
            CREATE TABLE IF NOT EXISTS caldav_sync_state (
                state_key TEXT PRIMARY KEY,
                state_json TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            -- Explicit local deletions must survive after the row itself is removed.
            -- The key format is E:<uid> for VEVENT and T:<uid> for VTODO.
            CREATE TABLE IF NOT EXISTS caldav_tombstones (
                item_key TEXT PRIMARY KEY,
                deleted_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            ",
        )?;

        let _ = self.conn.execute(
            "ALTER TABLE events ADD COLUMN urgency TEXT NOT NULL DEFAULT 'NORMAL_NOT_URGENT'",
            [],
        );

        Ok(())
    }

    fn seed_defaults_if_empty(&self) -> Result<()> {
        let count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM calendars", [], |row| row.get(0))?;
        if count == 0 {
            let work_id = Uuid::new_v4().to_string();
            let life_id = Uuid::new_v4().to_string();
            self.upsert_calendar(&Calendar {
                id: work_id.clone(),
                name: "工作日程".into(),
                color: "#2563EB".into(),
                account_type: "local".into(),
                is_visible: true,
                sync_url: None,
            })?;
            self.upsert_calendar(&Calendar {
                id: life_id.clone(),
                name: "个人生活".into(),
                color: "#10B981".into(),
                account_type: "local".into(),
                is_visible: true,
                sync_url: None,
            })?;

            let today = chrono::Local::now().date_naive();
            self.upsert_event(&Event {
                id: Uuid::new_v4().to_string(),
                calendar_id: work_id.clone(),
                summary: "项目核心里程碑评审 (重要紧急)".into(),
                description: "".into(),
                start_time: today.and_hms_opt(10, 0, 0).unwrap(),
                end_time: today.and_hms_opt(11, 0, 0).unwrap(),
                is_all_day: false,
                recurrence: Recurrence::None,
                exception_dates: vec![],
                reminder_minutes: Some(15),
                urgency: EventUrgency::ImportantUrgent,
            })?;

            self.upsert_event(&Event {
                id: Uuid::new_v4().to_string(),
                calendar_id: life_id,
                summary: "季度技术架构规划 (重要不紧急)".into(),
                description: "".into(),
                start_time: today.and_hms_opt(14, 0, 0).unwrap(),
                end_time: today.and_hms_opt(15, 30, 0).unwrap(),
                is_all_day: false,
                recurrence: Recurrence::None,
                exception_dates: vec![],
                reminder_minutes: Some(30),
                urgency: EventUrgency::ImportantNotUrgent,
            })?;

            self.upsert_todo(&Todo {
                id: Uuid::new_v4().to_string(),
                calendar_id: work_id,
                title: "完成 CalDAV 服务端联调测试".into(),
                description: "验证 Radicale / Nextcloud 双向同步".into(),
                priority: 1,
                completed: false,
                due_date: Some(today),
            })?;
        }
        Ok(())
    }


    /// Read the durable CalDAV synchronization baseline for an account/calendar.
    pub fn get_caldav_sync_state(&self, key: &str) -> Result<Option<String>> {
        let value = self.conn.query_row(
            "SELECT state_json FROM caldav_sync_state WHERE state_key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        ).optional()?;
        Ok(value)
    }

    /// Atomically replace the durable CalDAV synchronization baseline.
    pub fn set_caldav_sync_state(&self, key: &str, json: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO caldav_sync_state(state_key, state_json, updated_at)
             VALUES (?1, ?2, CURRENT_TIMESTAMP)
             ON CONFLICT(state_key) DO UPDATE SET
               state_json=excluded.state_json, updated_at=CURRENT_TIMESTAMP",
            params![key, json],
        )?;
        Ok(())
    }

    // --- Calendars ---
    pub fn list_calendars(&self) -> Result<Vec<Calendar>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, color, account_type, is_visible, sync_url FROM calendars",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Calendar {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                account_type: row.get(3)?,
                is_visible: row.get::<_, i64>(4)? != 0,
                sync_url: row.get(5)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn upsert_calendar(&self, cal: &Calendar) -> Result<()> {
        self.conn.execute(
            "INSERT INTO calendars (id, name, color, account_type, is_visible, sync_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               name=excluded.name,
               color=excluded.color,
               account_type=excluded.account_type,
               is_visible=excluded.is_visible,
               sync_url=excluded.sync_url",
            params![
                cal.id,
                cal.name,
                cal.color,
                cal.account_type,
                if cal.is_visible { 1 } else { 0 },
                cal.sync_url
            ],
        )?;
        Ok(())
    }

    pub fn delete_calendar(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM calendars WHERE id = ?1", params![id])?;
        Ok(())
    }

    // --- Events ---
    pub fn list_events(&self) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, calendar_id, summary, description, start_time, end_time,
                    is_all_day, recurrence, exception_dates, reminder_minutes, urgency
             FROM events ORDER BY start_time ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let start_str: String = row.get(4)?;
            let end_str: String = row.get(5)?;
            let rec_str: String = row.get(7)?;
            let exc_str: String = row.get(8)?;
            let urg_str: String = row.get::<_, Option<String>>(10)?.unwrap_or_default();

            let start_time = NaiveDateTime::parse_from_str(&start_str, "%Y-%m-%d %H:%M:%S")
                .unwrap_or_else(|_| chrono::Local::now().naive_local());
            let end_time = NaiveDateTime::parse_from_str(&end_str, "%Y-%m-%d %H:%M:%S")
                .unwrap_or(start_time);

            let exception_dates = exc_str
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .filter_map(|s| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok())
                .collect();

            Ok(Event {
                id: row.get(0)?,
                calendar_id: row.get(1)?,
                summary: row.get(2)?,
                description: row.get(3)?,
                start_time,
                end_time,
                is_all_day: row.get::<_, i64>(6)? != 0,
                recurrence: Recurrence::from_str(&rec_str),
                exception_dates,
                reminder_minutes: row.get(9)?,
                urgency: EventUrgency::from_str(&urg_str),
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn upsert_event(&self, ev: &Event) -> Result<()> {
        let exc_str = ev
            .exception_dates
            .iter()
            .map(|d| d.format("%Y-%m-%d").to_string())
            .collect::<Vec<_>>()
            .join(",");

        self.conn.execute(
            "INSERT INTO events (id, calendar_id, summary, description, start_time, end_time,
                                 is_all_day, recurrence, exception_dates, reminder_minutes, urgency)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
               calendar_id=excluded.calendar_id,
               summary=excluded.summary,
               description=excluded.description,
               start_time=excluded.start_time,
               end_time=excluded.end_time,
               is_all_day=excluded.is_all_day,
               recurrence=excluded.recurrence,
               exception_dates=excluded.exception_dates,
               reminder_minutes=excluded.reminder_minutes,
               urgency=excluded.urgency",
            params![
                ev.id,
                ev.calendar_id,
                ev.summary,
                ev.description,
                ev.start_time.format("%Y-%m-%d %H:%M:%S").to_string(),
                ev.end_time.format("%Y-%m-%d %H:%M:%S").to_string(),
                if ev.is_all_day { 1 } else { 0 },
                ev.recurrence.as_str(),
                exc_str,
                ev.reminder_minutes,
                ev.urgency.as_str(),
            ],
        )?;
        self.conn.execute("DELETE FROM caldav_tombstones WHERE item_key = ?1", params![format!("E:{}", ev.id)])?;
        Ok(())
    }

    /// 批量事务导入（单次 SQLite 事务 + 预编译语句，支持进度回调，数千条数据毫秒级写入）
    pub fn bulk_import_with_progress<F>(
        &mut self,
        calendars: &[Calendar],
        events: &[Event],
        todos: &[Todo],
        mut on_progress: F,
    ) -> Result<()>
    where
        F: FnMut(usize, usize),
    {
        let total = calendars.len() + events.len() + todos.len();
        let mut done = 0usize;
        on_progress(done, total);

        let tx = self.conn.transaction()?;

        if !calendars.is_empty() {
            let mut stmt = tx.prepare(
                "INSERT INTO calendars (id, name, color, account_type, is_visible, sync_url)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                   name=excluded.name,
                   color=excluded.color,
                   account_type=excluded.account_type,
                   is_visible=excluded.is_visible,
                   sync_url=excluded.sync_url",
            )?;
            for cal in calendars {
                stmt.execute(params![
                    cal.id,
                    cal.name,
                    cal.color,
                    cal.account_type,
                    if cal.is_visible { 1 } else { 0 },
                    cal.sync_url
                ])?;
                done += 1;
                if done % 50 == 0 || done == total {
                    on_progress(done, total);
                }
            }
        }

        if !events.is_empty() {
            let mut stmt = tx.prepare(
                "INSERT INTO events (id, calendar_id, summary, description, start_time, end_time,
                                     is_all_day, recurrence, exception_dates, reminder_minutes, urgency)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET
                   calendar_id=excluded.calendar_id,
                   summary=excluded.summary,
                   description=excluded.description,
                   start_time=excluded.start_time,
                   end_time=excluded.end_time,
                   is_all_day=excluded.is_all_day,
                   recurrence=excluded.recurrence,
                   exception_dates=excluded.exception_dates,
                   reminder_minutes=excluded.reminder_minutes,
                   urgency=excluded.urgency",
            )?;
            for ev in events {
                let exc_str = ev
                    .exception_dates
                    .iter()
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .collect::<Vec<_>>()
                    .join(",");

                stmt.execute(params![
                    ev.id,
                    ev.calendar_id,
                    ev.summary,
                    ev.description,
                    ev.start_time.format("%Y-%m-%d %H:%M:%S").to_string(),
                    ev.end_time.format("%Y-%m-%d %H:%M:%S").to_string(),
                    if ev.is_all_day { 1 } else { 0 },
                    ev.recurrence.as_str(),
                    exc_str,
                    ev.reminder_minutes,
                    ev.urgency.as_str(),
                ])?;
                tx.execute("DELETE FROM caldav_tombstones WHERE item_key=?1", params![format!("E:{}", ev.id)])?;
                done += 1;
                if done % 50 == 0 || done == total {
                    on_progress(done, total);
                }
            }
        }

        if !todos.is_empty() {
            let mut stmt = tx.prepare(
                "INSERT INTO todos (id, calendar_id, title, description, priority, completed, due_date)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                   calendar_id=excluded.calendar_id,
                   title=excluded.title,
                   description=excluded.description,
                   priority=excluded.priority,
                   completed=excluded.completed,
                   due_date=excluded.due_date",
            )?;
            for todo in todos {
                let due_str = todo.due_date.map(|d| d.format("%Y-%m-%d").to_string());
                stmt.execute(params![
                    todo.id,
                    todo.calendar_id,
                    todo.title,
                    todo.description,
                    todo.priority as i64,
                    if todo.completed { 1 } else { 0 },
                    due_str,
                ])?;
                tx.execute("DELETE FROM caldav_tombstones WHERE item_key=?1", params![format!("T:{}", todo.id)])?;
                done += 1;
                if done % 50 == 0 || done == total {
                    on_progress(done, total);
                }
            }
        }

        tx.commit()?;
        on_progress(total, total);
        Ok(())
    }

    /// User initiated deletion: persist a tombstone before removing the row.
    pub fn delete_event(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO caldav_tombstones(item_key) VALUES (?1) ON CONFLICT(item_key) DO UPDATE SET deleted_at=CURRENT_TIMESTAMP",
            params![format!("E:{}", id)],
        )?;
        self.conn.execute("DELETE FROM events WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Deletion caused by a successful remote sync must not create a local tombstone.
    pub fn delete_event_from_sync(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM events WHERE id = ?1", params![id])?;
        self.clear_caldav_tombstone(&format!("E:{}", id))
    }

    // --- Todos ---
    pub fn list_todos(&self) -> Result<Vec<Todo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, calendar_id, title, description, priority, completed, due_date
             FROM todos ORDER BY completed ASC, priority ASC, due_date ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let due_str: Option<String> = row.get(6)?;
            let due_date =
                due_str.and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok());
            Ok(Todo {
                id: row.get(0)?,
                calendar_id: row.get(1)?,
                title: row.get(2)?,
                description: row.get(3)?,
                priority: row.get::<_, i64>(4)? as u8,
                completed: row.get::<_, i64>(5)? != 0,
                due_date,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn upsert_todo(&self, todo: &Todo) -> Result<()> {
        let due_str = todo.due_date.map(|d| d.format("%Y-%m-%d").to_string());
        self.conn.execute(
            "INSERT INTO todos (id, calendar_id, title, description, priority, completed, due_date)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
               calendar_id=excluded.calendar_id,
               title=excluded.title,
               description=excluded.description,
               priority=excluded.priority,
               completed=excluded.completed,
               due_date=excluded.due_date",
            params![
                todo.id,
                todo.calendar_id,
                todo.title,
                todo.description,
                todo.priority as i64,
                if todo.completed { 1 } else { 0 },
                due_str,
            ],
        )?;
        self.conn.execute("DELETE FROM caldav_tombstones WHERE item_key = ?1", params![format!("T:{}", todo.id)])?;
        Ok(())
    }

    /// User initiated deletion: persist a tombstone before removing the row.
    pub fn delete_todo(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO caldav_tombstones(item_key) VALUES (?1) ON CONFLICT(item_key) DO UPDATE SET deleted_at=CURRENT_TIMESTAMP",
            params![format!("T:{}", id)],
        )?;
        self.conn.execute("DELETE FROM todos WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn delete_todo_from_sync(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM todos WHERE id = ?1", params![id])?;
        self.clear_caldav_tombstone(&format!("T:{}", id))
    }

    pub fn list_caldav_tombstones(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT item_key FROM caldav_tombstones ORDER BY deleted_at")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut out = Vec::new();
        for row in rows { out.push(row?); }
        Ok(out)
    }

    pub fn has_caldav_tombstone(&self, item_key: &str) -> Result<bool> {
        let exists: Option<i64> = self.conn.query_row(
            "SELECT 1 FROM caldav_tombstones WHERE item_key=?1", params![item_key], |row| row.get(0)
        ).optional()?;
        Ok(exists.is_some())
    }

    pub fn clear_caldav_tombstone(&self, item_key: &str) -> Result<()> {
        self.conn.execute("DELETE FROM caldav_tombstones WHERE item_key=?1", params![item_key])?;
        Ok(())
    }
}
