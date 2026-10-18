use super::{fonts::FontResolver, render_markdown_to_docx, template::render_with_template, ExportOptions};
use log::{info, warn};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Runtime};
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
    meeting_id: Option<String>,
) -> Result<Option<DocxExportResult>, String> {
    let options = ExportOptions { strip_sources: !keep_sources.unwrap_or(false), ..Default::default() };
    let fonts = FontResolver::detect();
    if !fonts.is_available() {
        info!("Font precheck unavailable on this platform; writing font names as-is");
    }
    let markdown = with_title(&markdown, &title);
    let template = read_template(&app)?;
    let with_template = template.is_some();
    let result = match template {
        Some(template) => render_with_template(&markdown, &template, &fonts, &options)?,
        None => render_markdown_to_docx(&markdown, &fonts, &options)?,
    };

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
    crate::audit::record(
        "export_docx",
        Some("meeting"),
        meeting_id.as_deref(),
        serde_json::json!({ "user_template": with_template, "keep_sources": !options.strip_sources }),
    )
    .await;
    Ok(Some(DocxExportResult { path: path.to_string_lossy().to_string(), warnings: result.warnings }))
}

/// The chosen template is copied into the app data folder, so moving or editing the original does not break export
fn template_paths<R: Runtime>(app: &AppHandle<R>) -> Result<(PathBuf, PathBuf), String> {
    let dir = app.path().app_data_dir().map_err(|e| format!("找不到应用数据目录：{}", e))?;
    Ok((dir.join("minutes_template.docx"), dir.join("minutes_template.name")))
}

fn read_template<R: Runtime>(app: &AppHandle<R>) -> Result<Option<Vec<u8>>, String> {
    let (file, _) = template_paths(app)?;
    if !file.exists() {
        return Ok(None);
    }
    std::fs::read(&file).map(Some).map_err(|e| format!("读取 Word 模板失败：{}", e))
}

#[derive(Debug, Serialize)]
pub struct MinutesTemplateInfo {
    pub name: String,
    /// What the template lacks for minutes, such as a missing heading style
    pub warnings: Vec<String>,
}

/// File name of the Word template used for export, or None for the built-in layout
#[tauri::command]
pub async fn get_minutes_template_command<R: Runtime>(app: AppHandle<R>) -> Result<Option<String>, String> {
    let (file, name) = template_paths(&app)?;
    if !file.exists() {
        return Ok(None);
    }
    Ok(Some(std::fs::read_to_string(&name).unwrap_or_else(|_| "自定义模板".to_string())))
}

/// Pick a .dotx / .docx template, check that minutes can be filled into it, and keep a copy.
/// Returns None when the user cancels the dialog.
#[tauri::command]
pub async fn choose_minutes_template_command<R: Runtime>(app: AppHandle<R>) -> Result<Option<MinutesTemplateInfo>, String> {
    let app_clone = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        app_clone.dialog().file().add_filter("Word 模板", &["dotx", "docx"]).blocking_pick_file()
    })
    .await
    .map_err(|e| format!("File dialog task failed: {}", e))?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let source = PathBuf::from(picked.to_string());
    let bytes = std::fs::read(&source).map_err(|e| format!("读取模板失败：{}", e))?;

    // A trial fill catches files that are not usable templates before they are saved
    let sample = "# 标题\n\n## 一级标题\n\n### 二级标题\n\n#### 三级标题\n\n正文";
    let trial = render_with_template(sample, &bytes, &FontResolver::detect(), &ExportOptions::default())?;

    let (file, name_file) = template_paths(&app)?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("保存模板失败：{}", e))?;
    }
    std::fs::write(&file, &bytes).map_err(|e| format!("保存模板失败：{}", e))?;
    let name = source.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "自定义模板".to_string());
    let _ = std::fs::write(&name_file, &name);
    info!("Minutes Word template set to {}", name);
    crate::audit::record("template_change", Some("setting"), Some("word_template"), serde_json::json!({ "template": "custom" })).await;
    Ok(Some(MinutesTemplateInfo { name, warnings: trial.warnings }))
}

/// Go back to the built-in layout
#[tauri::command]
pub async fn clear_minutes_template_command<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let (file, name) = template_paths(&app)?;
    for path in [file, name] {
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("删除模板失败：{}", e))?;
        }
    }
    crate::audit::record("template_change", Some("setting"), Some("word_template"), serde_json::json!({ "template": "builtin" })).await;
    Ok(())
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
