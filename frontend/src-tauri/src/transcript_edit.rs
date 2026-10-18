//! Editing a meeting's transcript line by line (记录稿逐条校订). The first edit of a line keeps
//! the transcribed text in `original_text`, so the line can be restored. A line that is right as
//! transcribed can be confirmed instead. Editing or confirming marks the line checked
//! (`reviewed_at`), so it leaves the to-check list; restoring puts it back. Every change is counted
//! in the audit log (lengths only, never the text).

use serde::Serialize;
use tauri::State;

use crate::database::repositories::meeting::MeetingsRepository;
use crate::state::AppState;

const FINALIZED_ERROR: &str = "纪要已定稿，记录稿不能修改；要修改请先取消定稿";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditedLine {
    id: String,
    text: String,
    /// The transcribed text, when the line differs from it
    original_text: Option<String>,
    reviewed: bool,
}

async fn ensure_editable(pool: &sqlx::SqlitePool, meeting_id: &str) -> Result<(), String> {
    match MeetingsRepository::get_finalized_at(pool, meeting_id).await {
        Ok(Some(_)) => Err(FINALIZED_ERROR.to_string()),
        Ok(None) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

struct Line {
    text: String,
    original_text: Option<String>,
    reviewed: bool,
}

async fn current(pool: &sqlx::SqlitePool, meeting_id: &str, line_id: &str) -> Result<Line, String> {
    sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT transcript, original_text, reviewed_at FROM transcripts WHERE id = ? AND meeting_id = ?",
    )
    .bind(line_id)
    .bind(meeting_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("读取记录稿失败：{}", e))?
    .map(|(text, original_text, reviewed_at)| Line { text, original_text, reviewed: reviewed_at.is_some() })
    .ok_or_else(|| "找不到这一句，可能已被重新转写，请刷新".to_string())
}

#[tauri::command]
pub async fn transcript_update_line(
    state: State<'_, AppState>,
    meeting_id: String,
    line_id: String,
    text: String,
) -> Result<EditedLine, String> {
    let pool = state.db_manager.pool();
    ensure_editable(pool, &meeting_id).await?;
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("内容不能为空；整句不要的话，可以只留一个句号".to_string());
    }
    let line = current(pool, &meeting_id, &line_id).await?;
    if text == line.text {
        return Ok(EditedLine { id: line_id, text, original_text: line.original_text, reviewed: line.reviewed });
    }
    let before = line.text;
    // Back to the transcribed text: the line no longer counts as edited, but it was checked
    let original = line.original_text.unwrap_or_else(|| before.clone());
    let original_text = (original != text).then_some(original);
    sqlx::query("UPDATE transcripts SET transcript = ?, original_text = ?, reviewed_at = ? WHERE id = ?")
        .bind(&text)
        .bind(&original_text)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(&line_id)
        .execute(pool)
        .await
        .map_err(|e| format!("保存失败：{}", e))?;
    crate::audit::record(
        "transcript_edit",
        Some("meeting"),
        Some(&meeting_id),
        serde_json::json!({ "line": line_id, "chars_before": before.chars().count(), "chars_after": text.chars().count() }),
    )
    .await;
    Ok(EditedLine { id: line_id, text, original_text, reviewed: true })
}

/// Marks a line as checked without changing it
#[tauri::command]
pub async fn transcript_confirm_line(
    state: State<'_, AppState>,
    meeting_id: String,
    line_id: String,
) -> Result<EditedLine, String> {
    let pool = state.db_manager.pool();
    ensure_editable(pool, &meeting_id).await?;
    let line = current(pool, &meeting_id, &line_id).await?;
    if !line.reviewed {
        sqlx::query("UPDATE transcripts SET reviewed_at = ? WHERE id = ?")
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(&line_id)
            .execute(pool)
            .await
            .map_err(|e| format!("保存失败：{}", e))?;
        crate::audit::record("transcript_confirm", Some("meeting"), Some(&meeting_id), serde_json::json!({ "line": line_id }))
            .await;
    }
    Ok(EditedLine { id: line_id, text: line.text, original_text: line.original_text, reviewed: true })
}

#[tauri::command]
pub async fn transcript_restore_line(
    state: State<'_, AppState>,
    meeting_id: String,
    line_id: String,
) -> Result<EditedLine, String> {
    let pool = state.db_manager.pool();
    ensure_editable(pool, &meeting_id).await?;
    let line = current(pool, &meeting_id, &line_id).await?;
    if line.original_text.is_none() && !line.reviewed {
        return Ok(EditedLine { id: line_id, text: line.text, original_text: None, reviewed: false });
    }
    // Also undoes a confirmation, for a line that was confirmed but not edited
    let original = line.original_text.unwrap_or(line.text);
    sqlx::query("UPDATE transcripts SET transcript = ?, original_text = NULL, reviewed_at = NULL WHERE id = ?")
        .bind(&original)
        .bind(&line_id)
        .execute(pool)
        .await
        .map_err(|e| format!("恢复失败：{}", e))?;
    crate::audit::record("transcript_restore", Some("meeting"), Some(&meeting_id), serde_json::json!({ "line": line_id }))
        .await;
    Ok(EditedLine { id: line_id, text: original, original_text: None, reviewed: false })
}
