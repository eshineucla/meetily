//! Glossary commands for Settings → 术语库, and for correcting an existing meeting

use std::path::PathBuf;

use serde::Serialize;
use serde_json::json;
use sqlx::Row;
use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::DialogExt;

use super::csv_io;
use super::matcher::{self, CorrectionHit, GlossaryEntry};

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    alias: String,
    canonicals: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryOverview {
    entries: Vec<GlossaryEntry>,
    conflicts: Vec<Conflict>,
    /// Terms that fit into the Whisper prompt, and how many there are
    prompt_terms: Vec<String>,
    prompt_terms_total: usize,
}

#[tauri::command]
pub async fn glossary_list() -> Result<GlossaryOverview, String> {
    let entries = super::load_entries().await?;
    let conflicts = matcher::detect_conflicts(&entries)
        .into_iter()
        .map(|(alias, canonicals)| Conflict { alias, canonicals })
        .collect();
    let (prompt_terms, prompt_terms_total) = matcher::prompt_terms(&entries, super::PROMPT_BUDGET_CHARS);
    Ok(GlossaryOverview { entries, conflicts, prompt_terms, prompt_terms_total })
}

async fn find_id(category: &str, canonical: &str) -> Result<Option<String>, String> {
    sqlx::query_scalar("SELECT id FROM glossary_entry WHERE category = ? AND canonical = ?")
        .bind(category)
        .bind(canonical)
        .fetch_optional(&super::pool()?)
        .await
        .map_err(|e| e.to_string())
}

/// Inserts, or updates the entry with the same id. Returns whether it was new.
async fn upsert(entry: &GlossaryEntry) -> Result<bool, String> {
    let pool = super::pool()?;
    let aliases = serde_json::to_string(&entry.aliases).map_err(|e| e.to_string())?;
    let ts = now();
    let updated = sqlx::query(
        "UPDATE glossary_entry SET category = ?, canonical = ?, aliases = ?, note = ?, enabled = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&entry.category)
    .bind(&entry.canonical)
    .bind(&aliases)
    .bind(&entry.note)
    .bind(entry.enabled as i64)
    .bind(&ts)
    .bind(&entry.id)
    .execute(&pool)
    .await
    .map_err(|e| format!("保存术语失败：{}", e))?;
    if updated.rows_affected() > 0 {
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO glossary_entry (id, category, canonical, aliases, note, enabled, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&entry.id)
    .bind(&entry.category)
    .bind(&entry.canonical)
    .bind(&aliases)
    .bind(&entry.note)
    .bind(entry.enabled as i64)
    .bind(&ts)
    .bind(&ts)
    .execute(&pool)
    .await
    .map_err(|e| format!("保存术语失败：{}", e))?;
    Ok(true)
}

/// Adds an entry (empty id) or saves changes to one
#[tauri::command]
pub async fn glossary_save(mut entry: GlossaryEntry) -> Result<GlossaryEntry, String> {
    entry.canonical = entry.canonical.trim().to_string();
    entry.note = entry.note.trim().to_string();
    entry.aliases = matcher::dedup_aliases(entry.aliases);
    let problems = matcher::validate_entry(&entry);
    if !problems.is_empty() {
        return Err(problems.join("；"));
    }
    if let Some(existing) = find_id(&entry.category, &entry.canonical).await? {
        if existing != entry.id {
            return Err(format!(
                "「{}」已经在{}里了，请直接修改那一条",
                entry.canonical,
                matcher::category_label(&entry.category).unwrap_or("术语库")
            ));
        }
    }
    if entry.id.is_empty() {
        entry.id = format!("gl-{}", uuid::Uuid::new_v4());
    }
    let added = upsert(&entry).await?;
    super::reload().await?;
    crate::audit::record(
        "glossary_change",
        Some("glossary"),
        Some(&entry.id),
        json!({ "op": if added { "add" } else { "update" }, "category": entry.category, "aliases": entry.aliases.len() }),
    )
    .await;
    Ok(entry)
}

#[tauri::command]
pub async fn glossary_set_enabled(id: String, enabled: bool) -> Result<(), String> {
    sqlx::query("UPDATE glossary_entry SET enabled = ?, updated_at = ? WHERE id = ?")
        .bind(enabled as i64)
        .bind(now())
        .bind(&id)
        .execute(&super::pool()?)
        .await
        .map_err(|e| e.to_string())?;
    super::reload().await?;
    crate::audit::record(
        "glossary_change",
        Some("glossary"),
        Some(&id),
        json!({ "op": if enabled { "enable" } else { "disable" } }),
    )
    .await;
    Ok(())
}

#[tauri::command]
pub async fn glossary_delete(id: String) -> Result<(), String> {
    sqlx::query("DELETE FROM glossary_entry WHERE id = ?")
        .bind(&id)
        .execute(&super::pool()?)
        .await
        .map_err(|e| e.to_string())?;
    super::reload().await?;
    crate::audit::record("glossary_change", Some("glossary"), Some(&id), json!({ "op": "delete" })).await;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    added: usize,
    updated: usize,
    problems: Vec<String>,
}

/// Reads a CSV file. A row whose category and correct spelling are already in the glossary
/// replaces that entry's misspellings, note and switch; other rows are added. Rows with
/// problems are skipped and listed; the good rows are still imported.
#[tauri::command]
pub async fn glossary_import_csv<R: Runtime>(app: AppHandle<R>) -> Result<Option<ImportReport>, String> {
    let picked = tokio::task::spawn_blocking(move || {
        app.dialog().file().add_filter("术语表", &["csv", "txt"]).blocking_pick_file()
    })
    .await
    .map_err(|e| format!("File dialog task failed: {}", e))?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = PathBuf::from(picked.to_string());
    let bytes = std::fs::read(&path).map_err(|e| format!("读取文件失败：{}", e))?;
    let (rows, problems) = csv_io::parse_csv(&bytes);

    let (mut added, mut updated) = (0, 0);
    for row in rows {
        let mut entry = row.entry;
        entry.id = match find_id(&entry.category, &entry.canonical).await? {
            Some(id) => id,
            None => format!("gl-{}", uuid::Uuid::new_v4()),
        };
        if upsert(&entry).await? {
            added += 1;
        } else {
            updated += 1;
        }
    }
    super::reload().await?;
    crate::audit::record(
        "glossary_import",
        Some("glossary"),
        None,
        json!({ "added": added, "updated": updated, "rejected": problems.len() }),
    )
    .await;
    Ok(Some(ImportReport { added, updated, problems }))
}

#[tauri::command]
pub async fn glossary_export_csv<R: Runtime>(app: AppHandle<R>) -> Result<Option<String>, String> {
    let entries = super::load_entries().await?;
    let default_name = format!("术语库-{}.csv", chrono::Local::now().format("%Y%m%d"));
    let picked = tokio::task::spawn_blocking(move || {
        app.dialog().file().set_file_name(&default_name).add_filter("术语表", &["csv"]).blocking_save_file()
    })
    .await
    .map_err(|e| format!("File dialog task failed: {}", e))?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = PathBuf::from(picked.to_string());
    std::fs::write(&path, csv_io::write_csv(&entries)).map_err(|e| format!("保存术语表失败：{}", e))?;
    crate::audit::record("glossary_export", Some("glossary"), None, json!({ "count": entries.len() })).await;
    Ok(Some(path.display().to_string()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Correction {
    text: String,
    hits: Vec<CorrectionHit>,
}

/// Tries the glossary on a piece of text without saving anything
#[tauri::command]
pub async fn glossary_preview(text: String) -> Result<Correction, String> {
    let (text, hits) = super::correct(&text);
    Ok(Correction { text, hits })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingCorrection {
    hits: Vec<CorrectionHit>,
    segments_changed: usize,
}

/// Applies the current glossary to a meeting's saved transcript, e.g. after adding entries
#[tauri::command]
pub async fn glossary_apply_to_meeting(meeting_id: String) -> Result<MeetingCorrection, String> {
    let pool = super::pool()?;
    if crate::database::repositories::meeting::MeetingsRepository::get_finalized_at(&pool, &meeting_id)
        .await
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("纪要已定稿，记录稿不能修改；要修改请先取消定稿".to_string());
    }
    let rows = sqlx::query("SELECT id, transcript FROM transcripts WHERE meeting_id = ?")
        .bind(&meeting_id)
        .fetch_all(&pool)
        .await
        .map_err(|e| format!("读取记录稿失败：{}", e))?;
    let originals: Vec<(String, String)> = rows.into_iter().map(|r| (r.get("id"), r.get("transcript"))).collect();
    let mut texts: Vec<String> = originals.iter().map(|(_, t)| t.clone()).collect();
    let (hits, segments_changed) = super::correct_all(texts.iter_mut());
    if segments_changed > 0 {
        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
        for ((id, before), after) in originals.iter().zip(&texts) {
            if before != after {
                sqlx::query("UPDATE transcripts SET transcript = ? WHERE id = ?")
                    .bind(after)
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| format!("保存记录稿失败：{}", e))?;
            }
        }
        tx.commit().await.map_err(|e| e.to_string())?;
        super::record_corrections(&meeting_id, &hits, segments_changed).await;
    }
    Ok(MeetingCorrection { hits, segments_changed })
}
