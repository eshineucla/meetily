//! Tauri commands for importing and exporting model packs (see [`super::pack`])

use std::path::PathBuf;

use log::info;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use super::pack::{self, CatalogFile, ImportReport, PackCandidate, Progress};
use crate::summary::summary_engine::ModelManagerState;

const PROGRESS_EVENT: &str = "model-pack-progress";
const PROGRESS_STEP: u64 = 16 * 1024 * 1024;

fn models_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| format!("无法确定数据目录：{}", e))?.join("models"))
}

fn catalog() -> Vec<CatalogFile> {
    let whisper: Vec<(&str, &str)> = crate::config::WHISPER_MODEL_CATALOG
        .iter()
        .map(|&(name, filename, ..)| (name, filename))
        .collect();
    let summary: Vec<(String, String, String, u64)> = crate::summary::summary_engine::get_available_models()
        .into_iter()
        .map(|m| (m.name, m.display_name, m.gguf_file, m.size_mb))
        .collect();
    let mut files = pack::whisper_files(&whisper);
    files.extend(pack::onnx_files(&crate::parakeet_engine::model_pack_files()));
    files.extend(pack::summary_files(&summary));
    files
}

/// Emits progress at most every 16 MiB, plus the last step
fn progress_emitter<R: Runtime>(app: AppHandle<R>, phase: &'static str) -> impl FnMut(Progress) {
    let mut last = 0u64;
    move |p: Progress| {
        if p.done_bytes >= last + PROGRESS_STEP || p.done_bytes == p.total_bytes {
            last = p.done_bytes;
            let _ = app.emit(
                PROGRESS_EVENT,
                serde_json::json!({
                    "phase": phase,
                    "doneBytes": p.done_bytes,
                    "totalBytes": p.total_bytes,
                    "current": p.current,
                }),
            );
        }
    }
}

/// Models on this machine that can be exported into a pack
#[tauri::command]
pub async fn model_pack_list_installed<R: Runtime>(app: AppHandle<R>) -> Result<Vec<PackCandidate>, String> {
    let dir = models_dir(&app)?;
    tokio::task::spawn_blocking(move || pack::installed_models(&dir, &catalog()))
        .await
        .map_err(|e| format!("读取模型列表失败：{}", e))
}

/// Pick a model pack (.zip) and install the model files in it.
/// Returns None when the user cancels the dialog.
#[tauri::command]
pub async fn model_pack_import<R: Runtime>(
    app: AppHandle<R>,
    summary_models: State<'_, ModelManagerState>,
) -> Result<Option<ImportReport>, String> {
    let app_clone = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        app_clone.dialog().file().add_filter("模型包", &["zip"]).blocking_pick_file()
    })
    .await
    .map_err(|e| format!("File dialog task failed: {}", e))?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let pack = PathBuf::from(picked.to_string());
    let dir = models_dir(&app)?;
    let progress = progress_emitter(app.clone(), "import");
    let report = tokio::task::spawn_blocking(move || pack::import_pack(&pack, &dir, &catalog(), progress))
        .await
        .map_err(|e| format!("导入模型包失败：{}", e))??;

    // Whisper and ONNX models are found by scanning on every listing; the summary manager caches
    let manager = summary_models.0.lock().await.clone();
    if let Some(manager) = manager {
        if let Err(e) = manager.scan_models().await {
            log::warn!("Failed to rescan summary models after import: {}", e);
        }
    }
    crate::audit::record(
        "model_import",
        None,
        None,
        serde_json::json!({ "imported": report.imported, "rejected": report.rejected.len(), "ignored": report.ignored.len() }),
    )
    .await;
    info!(
        "Model pack imported: {:?} (already present {:?}, ignored {}, rejected {})",
        report.imported,
        report.already_present,
        report.ignored.len(),
        report.rejected.len()
    );
    Ok(Some(report))
}

/// Write the chosen models into a pack (.zip) at a location the user picks.
/// Returns the saved path, or None when the user cancels the dialog.
#[tauri::command]
pub async fn model_pack_export<R: Runtime>(app: AppHandle<R>, model_ids: Vec<String>) -> Result<Option<String>, String> {
    if model_ids.is_empty() {
        return Err("没有选择模型".to_string());
    }
    let app_clone = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        app_clone
            .dialog()
            .file()
            .set_file_name("会议纪要-模型包.zip")
            .add_filter("模型包", &["zip"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| format!("File dialog task failed: {}", e))?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let destination = PathBuf::from(picked.to_string());
    let dir = models_dir(&app)?;
    let progress = progress_emitter(app.clone(), "export");
    let saved = destination.clone();
    let bytes = tokio::task::spawn_blocking(move || pack::export_pack(&dir, &catalog(), &model_ids, &destination, progress))
        .await
        .map_err(|e| format!("导出模型包失败：{}", e))??;
    info!("Model pack exported to {} ({} bytes)", saved.display(), bytes);
    crate::audit::record("model_export", None, None, serde_json::json!({ "bytes": bytes })).await;
    Ok(Some(saved.display().to_string()))
}
