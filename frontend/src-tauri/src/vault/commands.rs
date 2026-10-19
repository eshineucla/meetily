//! Data encryption status, key backup, and saving a meeting's recording in the clear

use super::{file_crypt, folders, KeyStorage};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    /// Where the data key is kept; None when it is not loaded
    key_storage: Option<KeyStorage>,
    message: String,
}

#[tauri::command]
pub async fn vault_status() -> Result<VaultStatus, String> {
    let key_storage = super::storage();
    let message = match key_storage {
        Some(KeyStorage::CredentialStore) => {
            "会议数据库、录音和记录稿文件已加密，密钥保存在系统凭据管理器中。建议导出一份密钥备份单独保管。".to_string()
        }
        Some(KeyStorage::File) => format!(
            "会议数据已加密，但系统凭据管理器不可用，密钥保存在应用数据文件夹的 {} 文件里，拿到这个文件夹就能解开数据。",
            super::KEY_FILE
        ),
        None => "数据密钥没有加载，会议数据无法打开。".to_string(),
    };
    Ok(VaultStatus { key_storage, message })
}

async fn save_dialog<R: Runtime>(app: &AppHandle<R>, file_name: String, filter: &'static str, extension: String) -> Result<Option<PathBuf>, String> {
    let app = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        app.dialog().file().set_file_name(&file_name).add_filter(filter, &[extension.as_str()]).blocking_save_file()
    })
    .await
    .map_err(|e| format!("File dialog task failed: {}", e))?;
    Ok(picked.map(|p| PathBuf::from(p.to_string())))
}

/// Writes a backup of the data key to a file the user picks. Returns its path, or None if cancelled.
#[tauri::command]
pub async fn vault_export_key<R: Runtime>(app: AppHandle<R>) -> Result<Option<String>, String> {
    let (Some(key), Some(dir)) = (super::key(), super::app_data_dir()) else {
        return Err("数据密钥没有加载".to_string());
    };
    let name = format!("会议纪要数据密钥备份-{}.txt", chrono::Local::now().format("%Y%m%d"));
    let Some(path) = save_dialog(&app, name, "文本文件", "txt".to_string()).await? else {
        return Ok(None);
    };
    std::fs::write(&path, super::backup_text(key, dir)).map_err(|e| format!("保存失败：{}", e))?;
    crate::audit::record("key_export", Some("setting"), Some("data_key"), serde_json::json!({})).await;
    Ok(Some(path.display().to_string()))
}

/// Saves a decrypted copy of the meeting's recording where the user picks, e.g. to play it
#[tauri::command]
pub async fn export_meeting_audio<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<Option<String>, String> {
    let (title, folder): (String, Option<String>) = sqlx::query_as("SELECT title, folder_path FROM meetings WHERE id = ?")
        .bind(&meeting_id)
        .fetch_optional(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "找不到这场会议".to_string())?;
    let folder = PathBuf::from(folder.ok_or_else(|| "这场会议没有录音".to_string())?);
    let source = crate::audio::retranscription::find_audio_file(&folder).map_err(|_| "这场会议没有录音".to_string())?;

    let plain_name = file_crypt::plain_path(&source).unwrap_or_else(|| source.clone());
    let extension = plain_name.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_string();
    let safe_title: String = title.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let Some(mut target) = save_dialog(&app, format!("{}.{}", safe_title.trim(), extension), "录音", extension.clone()).await? else {
        return Ok(None);
    };
    if target.extension().is_none() {
        target.set_extension(&extension);
    }
    let dst = target.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let copy = folders::readable(&source).map_err(|e| e.to_string())?;
        std::fs::copy(copy.path(), &dst).map(|_| ()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("导出录音失败：{}", e))?;
    crate::audit::record_meeting("export_audio", &meeting_id, serde_json::json!({})).await;
    Ok(Some(target.display().to_string()))
}
