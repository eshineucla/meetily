//! Import an existing transcript (pasted text or a .txt/.md/.srt/.vtt file) as a meeting,
//! so it can be summarized without recording or transcribing audio.

use crate::api::TranscriptSegment;
use crate::database::repositories::transcript::TranscriptsRepository;
use crate::state::AppState;
use log::{info, warn};
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

pub const TRANSCRIPT_EXTENSIONS: &[&str] = &["txt", "md", "srt", "vtt"];

/// Larger files are almost certainly not a meeting transcript
const MAX_TRANSCRIPT_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct TranscriptFile {
    pub path: String,
    /// File name without extension, used as the default meeting title
    pub name: String,
    pub content: String,
}

/// Decode file bytes as UTF-8 (with or without BOM) or UTF-16 (with BOM),
/// falling back to GBK, which Chinese Windows tools such as Notepad still produce.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some((encoding, bom_len)) = encoding_rs::Encoding::for_bom(bytes) {
        let (text, _) = encoding.decode_without_bom_handling(&bytes[bom_len..]);
        return text.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => {
            let (text, _, _) = encoding_rs::GBK.decode(bytes);
            text.into_owned()
        }
    }
}

/// Split a transcript into segments, one per non-empty line.
/// Subtitle cue numbers, timing lines and the WEBVTT header are dropped.
pub fn split_into_segments(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.chars().all(|c| c.is_ascii_digit()))
        .filter(|line| !line.contains("-->"))
        .filter(|line| !line.starts_with("WEBVTT"))
        .map(str::to_string)
        .collect()
}

fn read_transcript_file(path: &Path) -> Result<TranscriptFile, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if !TRANSCRIPT_EXTENSIONS.contains(&ext.as_str()) {
        return Err(format!(
            "Unsupported file type '.{}'. Supported: {}",
            ext,
            TRANSCRIPT_EXTENSIONS.join(", ")
        ));
    }

    let size = std::fs::metadata(path)
        .map_err(|e| format!("Failed to read '{}': {}", path.display(), e))?
        .len();
    if size > MAX_TRANSCRIPT_BYTES {
        return Err(format!("File is too large ({} MB, max 10 MB)", size / 1024 / 1024));
    }

    let bytes = std::fs::read(path).map_err(|e| format!("Failed to read '{}': {}", path.display(), e))?;
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    Ok(TranscriptFile {
        path: path.to_string_lossy().to_string(),
        name,
        content: decode_text(&bytes),
    })
}

/// Open a file dialog and read the chosen transcript file
#[tauri::command]
pub async fn select_transcript_file_command<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<TranscriptFile>, String> {
    let app_clone = app.clone();
    let file_path = tokio::task::spawn_blocking(move || {
        app_clone
            .dialog()
            .file()
            .add_filter("Transcript Files", TRANSCRIPT_EXTENSIONS)
            .blocking_pick_file()
    })
    .await
    .map_err(|e| format!("File dialog task failed: {}", e))?;

    match file_path {
        Some(path) => read_transcript_file(Path::new(&path.to_string())).map(Some),
        None => Ok(None),
    }
}

/// Read a transcript file from a given path (for drag-drop)
#[tauri::command]
pub async fn read_transcript_file_command(path: String) -> Result<TranscriptFile, String> {
    read_transcript_file(Path::new(&path))
}

/// Save the transcript text as a new meeting and return its id
#[tauri::command]
pub async fn import_transcript_text_command(
    state: State<'_, AppState>,
    title: String,
    text: String,
) -> Result<String, String> {
    let lines = split_into_segments(&text);
    if lines.is_empty() {
        return Err("The transcript is empty".to_string());
    }

    let title = match title.trim() {
        "" => format!("Imported transcript {}", chrono::Local::now().format("%Y-%m-%d %H:%M")),
        t => t.to_string(),
    };

    let timestamp = chrono::Utc::now().to_rfc3339();
    let mut segments: Vec<TranscriptSegment> = lines
        .into_iter()
        .map(|line| TranscriptSegment {
            id: format!("transcript-{}", Uuid::new_v4()),
            text: line,
            timestamp: timestamp.clone(),
            audio_start_time: None,
            audio_end_time: None,
            duration: None,
            confidence: None,
        })
        .collect();
    let (glossary_hits, glossary_changed) = crate::glossary::correct_all(segments.iter_mut().map(|s| &mut s.text));

    let meeting_id = TranscriptsRepository::save_transcript(state.db_manager.pool(), &title, &segments, None)
        .await
        .map_err(|e| {
            warn!("Failed to save imported transcript '{}': {}", title, e);
            format!("Failed to save transcript: {}", e)
        })?;

    info!("Imported transcript '{}' as {} ({} segments)", title, meeting_id, segments.len());
    crate::audit::record_meeting("import_text", &meeting_id, serde_json::json!({ "segments": segments.len() })).await;
    crate::glossary::record_corrections(&meeting_id, &glossary_hits, glossary_changed).await;
    Ok(meeting_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8_with_and_without_bom() {
        assert_eq!(decode_text("会议记录".as_bytes()), "会议记录");
        let mut with_bom = vec![0xEF, 0xBB, 0xBF];
        with_bom.extend_from_slice("会议记录".as_bytes());
        assert_eq!(decode_text(&with_bom), "会议记录");
    }

    #[test]
    fn decodes_gbk() {
        let (gbk, _, _) = encoding_rs::GBK.encode("会议记录");
        assert_eq!(decode_text(&gbk), "会议记录");
    }

    #[test]
    fn decodes_utf16le_with_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "会议".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_text(&bytes), "会议");
    }

    #[test]
    fn splits_lines_and_drops_subtitle_markup() {
        let srt = "1\n00:00:01,000 --> 00:00:03,000\n张三：大家好。\n\n2\n00:00:03,000 --> 00:00:05,000\n李四：开始吧。\n";
        assert_eq!(split_into_segments(srt), vec!["张三：大家好。", "李四：开始吧。"]);
        assert_eq!(split_into_segments("WEBVTT\n\n第一段\n  \n第二段"), vec!["第一段", "第二段"]);
    }
}
