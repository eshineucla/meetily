//! Audit trail commands for Settings → 审计日志: browse, verify, export a report for a security
//! inspection, and show whether the data drive is encrypted (BitLocker).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_dialog::DialogExt;

use super::chain::{self, AuditRow, ChainProblem};

#[derive(Serialize)]
pub struct AuditEntry {
    #[serde(flatten)]
    row: AuditRow,
    label: &'static str,
}

#[derive(Serialize)]
pub struct AuditPage {
    entries: Vec<AuditEntry>,
    total: usize,
}

fn label(action: &str) -> &'static str {
    chain::action_label(action).unwrap_or("未登记的动作")
}

/// Rows in the period, newest first, one page at a time
#[tauri::command]
pub async fn audit_list(
    from: Option<String>,
    to: Option<String>,
    action: Option<String>,
    offset: usize,
    limit: usize,
) -> Result<AuditPage, String> {
    let mut rows = super::rows_between(from.as_deref(), to.as_deref()).await?;
    if let Some(action) = action.filter(|a| !a.is_empty()) {
        rows.retain(|r| r.action == action);
    }
    rows.reverse();
    let total = rows.len();
    let entries = rows
        .into_iter()
        .skip(offset)
        .take(limit.min(500))
        .map(|row| AuditEntry { label: label(&row.action), row })
        .collect();
    Ok(AuditPage { entries, total })
}

#[derive(Serialize)]
pub struct DiskEncryption {
    /// "on", "off", "in_progress", "unknown"
    state: &'static str,
    drive: String,
    message: String,
}

#[derive(Serialize)]
pub struct AuditStatus {
    total_rows: usize,
    integrity_ok: bool,
    problems: Vec<ChainProblem>,
    /// check item → "ok" when at least one of its actions was recorded, else "empty"
    check_items: BTreeMap<&'static str, &'static str>,
    actions: Vec<(&'static str, &'static str)>,
    disk: DiskEncryption,
}

fn check_items(rows: &[AuditRow]) -> BTreeMap<&'static str, &'static str> {
    chain::CHECK_ITEMS
        .iter()
        .map(|(item, actions)| {
            let seen = rows.iter().any(|r| actions.contains(&r.action.as_str()));
            (*item, if seen { "ok" } else { "empty" })
        })
        .collect()
}

/// Verifies the whole chain and reports the drive encryption state
#[tauri::command]
pub async fn audit_status<R: Runtime>(app: AppHandle<R>) -> Result<AuditStatus, String> {
    let rows = super::rows_between(None, None).await?;
    let problems = chain::verify_rows(&rows);
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let disk = tokio::task::spawn_blocking(move || disk_encryption(&data_dir))
        .await
        .map_err(|e| e.to_string())?;
    Ok(AuditStatus {
        total_rows: rows.len(),
        integrity_ok: problems.is_empty(),
        check_items: check_items(&rows),
        problems,
        actions: chain::ACTIONS.to_vec(),
        disk,
    })
}

/// Writes the report for the period to a JSON file the user picks. Returns its path, or None
/// when the dialog is cancelled. The integrity result covers the whole log, not only the period.
#[tauri::command]
pub async fn audit_export_report<R: Runtime>(
    app: AppHandle<R>,
    from: Option<String>,
    to: Option<String>,
) -> Result<Option<String>, String> {
    let all = super::rows_between(None, None).await?;
    let problems = chain::verify_rows(&all);
    let rows = super::rows_between(from.as_deref(), to.as_deref()).await?;

    let mut operators: BTreeMap<&str, usize> = BTreeMap::new();
    let mut actions: BTreeMap<&str, usize> = BTreeMap::new();
    let mut results: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &rows {
        *operators.entry(&r.operator).or_default() += 1;
        *actions.entry(&r.action).or_default() += 1;
        *results.entry(&r.result).or_default() += 1;
    }
    let mut report = json!({
        "generated_at": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "generated_by": super::operator(),
        "app_version": env!("CARGO_PKG_VERSION"),
        "classified_build": !cfg!(feature = "online"),
        "period": { "from": from, "to": to },
        "integrity": {
            "result": if problems.is_empty() { "pass" } else { "fail" },
            "checked_rows": all.len(),
            "head_hash": all.last().map(|r| r.row_hash.clone()),
            "problems": problems,
        },
        "summary": {
            "total": rows.len(),
            "operators": operators,
            "actions": actions,
            "results": results,
        },
        "check_items": check_items(&all),
        "action_labels": chain::ACTIONS.iter().map(|(a, l)| (a.to_string(), l.to_string())).collect::<BTreeMap<_, _>>(),
        "records": rows,
    });
    if !problems.is_empty() {
        report["action_required"] = json!("完整性校验未通过：立即停止使用本系统，封存设备并上报保密机构，不得自行修复。");
    }

    let default_name = format!("审计报告-{}.json", chrono::Local::now().format("%Y%m%d-%H%M"));
    let app_clone = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        app_clone.dialog().file().set_file_name(&default_name).add_filter("审计报告", &["json"]).blocking_save_file()
    })
    .await
    .map_err(|e| format!("File dialog task failed: {}", e))?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = PathBuf::from(picked.to_string());
    let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("保存审计报告失败：{}", e))?;
    super::record(
        "audit_export",
        None,
        None,
        json!({ "rows": rows.len(), "integrity": if problems.is_empty() { "pass" } else { "fail" } }),
    )
    .await;
    Ok(Some(path.display().to_string()))
}

/// BitLocker state of the drive holding the app data, read through the shell's
/// System.Volume.BitLockerProtection property, which needs no administrator rights
#[cfg(target_os = "windows")]
fn disk_encryption(data_dir: &Path) -> DiskEncryption {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let drive = data_dir
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().trim_end_matches('\\').to_string())
        .unwrap_or_else(|| "C:".to_string());
    let script = format!(
        "(New-Object -ComObject Shell.Application).NameSpace('{}\\').Self.ExtendedProperty('System.Volume.BitLockerProtection')",
        drive.replace('\'', "")
    );
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let value = output
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<i32>().ok());
    let (state, message) = match value {
        Some(1) | Some(6) => ("on", format!("{} 盘已启用 BitLocker", drive)),
        Some(3) => ("in_progress", format!("{} 盘正在启用 BitLocker", drive)),
        Some(8) => (
            "off",
            format!("{} 盘的 BitLocker 等待激活，数据尚未受保护。请在“管理 BitLocker”中完成启用", drive),
        ),
        Some(2) | Some(4) | Some(5) => (
            "off",
            format!("{} 盘未启用 BitLocker，会议数据以明文存放。请启用 BitLocker 或单位统一的磁盘加密", drive),
        ),
        _ => ("unknown", format!("无法读取 {} 盘的 BitLocker 状态，请人工确认磁盘已加密", drive)),
    };
    DiskEncryption { state, drive, message }
}

#[cfg(not(target_os = "windows"))]
fn disk_encryption(data_dir: &Path) -> DiskEncryption {
    DiskEncryption {
        state: "unknown",
        drive: data_dir.display().to_string(),
        message: "此系统不检查磁盘加密，请人工确认已启用磁盘加密（如 FileVault）".to_string(),
    }
}
