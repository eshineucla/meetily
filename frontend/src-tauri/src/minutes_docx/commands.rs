use super::{fonts::FontResolver, render_markdown_to_docx, ExportOptions};
use log::{info, warn};
use serde::Serialize;
use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::DialogExt;

#[derive(Debug, Serialize)]
pub struct DocxExportResult {
    pub path: String,
    /// Layout differences the user should know about, such as a substituted font
    pub warnings: Vec<String>,
}

/// Characters Windows does not allow in file names
fn safe_file_name(title: &str) -> String {
    let name: String = title
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let name = name.trim().trim_end_matches('.').to_string();
    if name.is_empty() { "会议纪要".to_string() } else { name }
}

/// The minutes need a document title; use the meeting title when the markdown has none
fn with_title(markdown: &str, title: &str) -> String {
    let has_title = markdown.lines().find(|l| !l.trim().is_empty()).is_some_and(|l| l.trim_start().starts_with("# "));
    if has_title || title.trim().is_empty() {
        markdown.to_string()
    } else {
        format!("# {}\n\n{}", title.trim(), markdown)
    }
}

/// Ask where to save, then write the minutes as an official-document style .docx.
/// Returns None when the user cancels the dialog.
#[tauri::command]
pub async fn export_minutes_docx_command<R: Runtime>(
    app: AppHandle<R>,
    title: String,
    markdown: String,
    keep_sources: Option<bool>,
) -> Result<Option<DocxExportResult>, String> {
    let options = ExportOptions { strip_sources: !keep_sources.unwrap_or(false), ..Default::default() };
    let fonts = FontResolver::detect();
    if !fonts.is_available() {
        info!("Font precheck unavailable on this platform; writing font names as-is");
    }
    let result = render_markdown_to_docx(&with_title(&markdown, &title), &fonts, &options)?;

    let file_name = format!("{}.docx", safe_file_name(&title));
    let app_clone = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        app_clone.dialog().file().set_file_name(&file_name).add_filter("Word 文档", &["docx"]).blocking_save_file()
    })
    .await
    .map_err(|e| format!("File dialog task failed: {}", e))?;

    let Some(picked) = picked else {
        return Ok(None);
    };
    let mut path = std::path::PathBuf::from(picked.to_string());
    if path.extension().is_none() {
        path.set_extension("docx");
    }
    std::fs::write(&path, &result.bytes).map_err(|e| format!("保存失败：{}", e))?;

    for warning in &result.warnings {
        warn!("docx export: {}", warning);
    }
    info!("Exported minutes to {}", path.display());
    Ok(Some(DocxExportResult { path: path.to_string_lossy().to_string(), warnings: result.warnings }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_made_safe() {
        assert_eq!(safe_file_name("关于A/B的会议: 纪要?"), "关于A_B的会议_ 纪要_");
        assert_eq!(safe_file_name("  "), "会议纪要");
    }

    #[test]
    fn title_is_added_only_when_missing() {
        assert_eq!(with_title("## 会议概况\n内容", "预算会"), "# 预算会\n\n## 会议概况\n内容");
        assert_eq!(with_title("\n# 已有标题\n正文", "预算会"), "\n# 已有标题\n正文");
    }
}
