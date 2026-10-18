//! Audit trail (M3-05): who did what, when, to which meeting, recorded in the app database and
//! protected against editing (see [`chain`]). Commands call [`record`]; it never fails the
//! operation it records, but a refused or failed write is logged as an error.

pub mod chain;
pub mod commands;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use once_cell::sync::Lazy;
use serde_json::Value;
use sqlx::{Row, SqlitePool};
use tokio::sync::Mutex;

use chain::AuditRow;

static POOL: RwLock<Option<SqlitePool>> = RwLock::new(None);
static STARTED: AtomicBool = AtomicBool::new(false);
/// Reading the last hash and inserting the next row must not interleave
static WRITE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Called whenever the database is opened and migrated; the first call records the app start
pub fn attach_database(pool: SqlitePool) {
    *POOL.write().unwrap() = Some(pool);
    if !STARTED.swap(true, Ordering::SeqCst) {
        tauri::async_runtime::spawn(record("app_start", None, None, serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "classified": !cfg!(feature = "online"),
        })));
    }
}

fn pool() -> Option<SqlitePool> {
    POOL.read().unwrap().clone()
}

/// The signed-in operating system account
pub fn operator() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Records a successful action. `detail` holds ids and counts only, never meeting content.
pub async fn record(action: &'static str, target_type: Option<&str>, target_id: Option<&str>, detail: Value) {
    write(action, target_type, target_id, detail, "success").await;
}

/// Records an action that failed
pub async fn record_failure(action: &'static str, target_type: Option<&str>, target_id: Option<&str>, detail: Value) {
    write(action, target_type, target_id, detail, "failure").await;
}

/// For a meeting
pub async fn record_meeting(action: &'static str, meeting_id: &str, detail: Value) {
    record(action, Some("meeting"), Some(meeting_id), detail).await;
}

async fn write(action: &'static str, target_type: Option<&str>, target_id: Option<&str>, detail: Value, result: &str) {
    if let Err(e) = try_write(action, target_type, target_id, &detail, result).await {
        log::error!("Audit record '{}' was not written: {}", action, e);
    }
}

async fn try_write(
    action: &str,
    target_type: Option<&str>,
    target_id: Option<&str>,
    detail: &Value,
    result: &str,
) -> Result<(), String> {
    if chain::action_label(action).is_none() {
        return Err(format!("unregistered audit action {}", action));
    }
    chain::check_detail_safety(detail)?;
    let pool = pool().ok_or("database not open yet")?;
    let detail_json = chain::detail_to_json(detail);
    let operator = operator();
    let ts = now();

    let _guard = WRITE_LOCK.lock().await;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let prev: Option<String> = sqlx::query_scalar("SELECT row_hash FROM audit_log ORDER BY log_id DESC LIMIT 1")
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let prev = prev.unwrap_or_else(|| chain::GENESIS_HASH.to_string());
    let row_hash = chain::compute_row_hash(Some(&prev), &ts, &operator, action, target_type, target_id, &detail_json, result);
    sqlx::query(
        "INSERT INTO audit_log (ts, operator, action, target_type, target_id, detail, result, prev_hash, row_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&ts)
    .bind(&operator)
    .bind(action)
    .bind(target_type)
    .bind(target_id)
    .bind(&detail_json)
    .bind(result)
    .bind(&prev)
    .bind(&row_hash)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())
}

/// Rows with `ts` in [from, to] (either may be open), oldest first
pub async fn rows_between(from: Option<&str>, to: Option<&str>) -> Result<Vec<AuditRow>, String> {
    let pool = pool().ok_or("数据库尚未打开")?;
    let rows = sqlx::query(
        "SELECT log_id, ts, operator, action, target_type, target_id, detail, result, prev_hash, row_hash \
         FROM audit_log WHERE (? IS NULL OR ts >= ?) AND (? IS NULL OR ts <= ?) ORDER BY log_id ASC",
    )
    .bind(from)
    .bind(from)
    .bind(to)
    .bind(to)
    .fetch_all(&pool)
    .await
    .map_err(|e| format!("读取审计日志失败：{}", e))?;
    Ok(rows
        .into_iter()
        .map(|r| AuditRow {
            log_id: r.get("log_id"),
            ts: r.get("ts"),
            operator: r.get("operator"),
            action: r.get("action"),
            target_type: r.get("target_type"),
            target_id: r.get("target_id"),
            detail: r.get("detail"),
            result: r.get("result"),
            prev_hash: r.get("prev_hash"),
            row_hash: r.get("row_hash"),
        })
        .collect())
}
