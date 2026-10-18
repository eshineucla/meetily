//! Audit trail rules, ported from the earlier workbench (`mmwf/infra/audit.py`). This module
//! has no database or Tauri dependency so it can be tested on its own.
//!
//! The log is append-only in two independent ways:
//! 1. SQLite triggers reject UPDATE and DELETE on `audit_log` (migration `add_audit_log`).
//! 2. Every row carries `row_hash = sha256(prev|ts|operator|action|target_type|target_id|detail|result)`,
//!    with `prev` the stored `row_hash` of the row before it. Editing the database file directly
//!    bypasses the triggers but breaks the chain at the edited row.
//!
//! `detail` never holds meeting content (transcripts, minutes, names): the log is handed to
//! security inspections, so it records ids and short facts only. [`check_detail_safety`]
//! refuses the common mistakes before anything is written, since nothing can be removed later.

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// prev_hash of the first row
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Every action the app records. A new action is added here, with its check item below.
pub const ACTIONS: &[(&str, &str)] = &[
    ("app_start", "启动程序"),
    ("app_exit", "退出程序"),
    ("record_start", "开始录音"),
    ("record_stop", "结束录音"),
    ("meeting_create", "保存会议"),
    ("import_audio", "导入音频"),
    ("import_text", "导入记录稿"),
    ("summary_generate", "生成纪要"),
    ("summary_edit", "修改纪要"),
    ("summary_restore", "恢复纪要版本"),
    ("meeting_rename", "修改会议名称"),
    ("minutes_finalize", "定稿"),
    ("minutes_unfinalize", "取消定稿"),
    ("export_docx", "导出 Word"),
    ("meeting_delete", "删除会议"),
    ("template_change", "更换导出模板"),
    ("settings_change", "修改模型或转写设置"),
    ("model_import", "导入模型包"),
    ("model_export", "导出模型包"),
    ("audit_export", "导出审计报告"),
];

/// Check items an inspection looks for, and the actions that cover each
pub const CHECK_ITEMS: &[(&str, &[&str])] = &[
    ("使用记录", &["app_start", "app_exit"]),
    ("会议内容进入记录", &["record_start", "record_stop", "meeting_create", "import_audio", "import_text"]),
    ("纪要生成记录", &["summary_generate"]),
    ("文稿修改记录", &["summary_edit", "summary_restore", "meeting_rename"]),
    ("定稿记录", &["minutes_finalize", "minutes_unfinalize"]),
    ("导出记录", &["export_docx", "audit_export"]),
    ("删除记录", &["meeting_delete"]),
    ("配置变更记录", &["template_change", "settings_change", "model_import", "model_export"]),
];

pub fn action_label(action: &str) -> Option<&'static str> {
    ACTIONS.iter().find(|(a, _)| *a == action).map(|(_, label)| *label)
}

/// Keys that by their name carry content
const FORBIDDEN_DETAIL_KEYS: &[&str] = &[
    "text", "raw_text", "original_text", "content", "body", "full_text", "transcript",
    "transcript_text", "paragraph", "minutes", "minutes_text", "plain", "plain_text", "summary",
    "markdown", "title", "name", "meeting_name", "原文", "正文", "全文", "转写稿", "纪要正文",
];

/// Keys that are fine with a count but not with a list or object
const FORBIDDEN_CONTAINER_KEYS: &[&str] = &[
    "segments", "segment_list", "paragraphs", "topics", "resolutions", "action_items", "lines",
    "sentences", "chunks", "记录", "分段",
];

/// Longer strings are content, not identifiers
pub const MAX_DETAIL_STRING_LEN: usize = 256;
const MAX_DETAIL_NODES: usize = 500;

/// Canonical JSON for `detail`: sorted keys, no spaces, UTF-8 kept as is. Matches Python's
/// `json.dumps(sort_keys=True, ensure_ascii=False, separators=(",", ":"))` as used by the
/// earlier workbench. Keys are sorted here because serde_json keeps insertion order when another
/// crate turns on its `preserve_order` feature.
pub fn detail_to_json(detail: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let mut out = serde_json::Map::new();
                for key in keys {
                    out.insert(key.clone(), sorted(&map[key]));
                }
                Value::Object(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    if detail.is_null() {
        return "{}".to_string();
    }
    sorted(detail).to_string()
}

pub fn check_detail_safety(detail: &Value) -> Result<(), String> {
    let mut violations = Vec::new();
    let mut nodes = 0usize;
    fn walk(node: &Value, path: &str, nodes: &mut usize, violations: &mut Vec<String>) -> Result<(), String> {
        *nodes += 1;
        if *nodes > MAX_DETAIL_NODES {
            return Err("审计详情结构过大，只应记录标识符".to_string());
        }
        match node {
            Value::Object(map) => {
                for (key, value) in map {
                    let lower = key.to_lowercase();
                    let child = format!("{}.{}", path, key);
                    if FORBIDDEN_DETAIL_KEYS.contains(&lower.as_str()) {
                        violations.push(format!("{} 是禁止的键名（会装正文）", child));
                    } else if FORBIDDEN_CONTAINER_KEYS.contains(&lower.as_str())
                        && (value.is_array() || value.is_object())
                    {
                        violations.push(format!("{} 是列表或对象，疑似整段内容", child));
                    }
                    walk(value, &child, nodes, violations)?;
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    walk(item, &format!("{}[{}]", path, i), nodes, violations)?;
                }
            }
            Value::String(s) if s.chars().count() > MAX_DETAIL_STRING_LEN => {
                violations.push(format!("{} 长度超过 {}，疑似正文", path, MAX_DETAIL_STRING_LEN));
            }
            _ => {}
        }
        Ok(())
    }
    walk(detail, "detail", &mut nodes, &mut violations)?;
    if violations.is_empty() {
        Ok(())
    } else {
        Err(format!("审计详情含疑似正文，已拒绝写入：{}", violations.join("；")))
    }
}

#[allow(clippy::too_many_arguments)]
pub fn compute_row_hash(
    prev_hash: Option<&str>,
    ts: &str,
    operator: &str,
    action: &str,
    target_type: Option<&str>,
    target_id: Option<&str>,
    detail_json: &str,
    result: &str,
) -> String {
    let raw = [
        prev_hash.unwrap_or(GENESIS_HASH),
        ts,
        operator,
        action,
        target_type.unwrap_or(""),
        target_id.unwrap_or(""),
        detail_json,
        result,
    ]
    .join("|");
    hex(&Sha256::digest(raw.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// One stored row
#[derive(Debug, Clone, Serialize)]
pub struct AuditRow {
    pub log_id: i64,
    pub ts: String,
    pub operator: String,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub detail: String,
    pub result: String,
    pub prev_hash: Option<String>,
    pub row_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChainProblem {
    pub log_id: i64,
    pub kind: &'static str,
    pub explanation: &'static str,
}

/// Checks rows in log_id order. Each row is compared with its own fields and with the row
/// hash stored on the row before it (not a recomputed one), so a single edited row is reported
/// once instead of making every later row look broken.
pub fn verify_rows(rows: &[AuditRow]) -> Vec<ChainProblem> {
    let mut problems = Vec::new();
    let mut prev_stored: Option<&str> = None;
    for row in rows {
        let recomputed = compute_row_hash(
            row.prev_hash.as_deref(),
            &row.ts,
            &row.operator,
            &row.action,
            row.target_type.as_deref(),
            row.target_id.as_deref(),
            &row.detail,
            &row.result,
        );
        if recomputed != row.row_hash {
            problems.push(ChainProblem {
                log_id: row.log_id,
                kind: "row_hash_mismatch",
                explanation: "本行内容与哈希不符（该行被改过）",
            });
        }
        let prev = row.prev_hash.as_deref().unwrap_or(GENESIS_HASH);
        let expected_prev = prev_stored.unwrap_or(GENESIS_HASH);
        if prev != expected_prev {
            problems.push(ChainProblem {
                log_id: row.log_id,
                kind: "prev_hash_mismatch",
                explanation: "与上一行的哈希接不上（中间有行被删除、替换或插入）",
            });
        }
        prev_stored = Some(&row.row_hash);
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(log_id: i64, prev: Option<&str>, action: &str) -> AuditRow {
        let ts = format!("2026-10-18T08:00:0{}Z", log_id);
        let detail = detail_to_json(&json!({"meeting_id": "m-1", "words": 12}));
        let row_hash = compute_row_hash(prev, &ts, "user", action, Some("meeting"), Some("m-1"), &detail, "success");
        AuditRow {
            log_id,
            ts,
            operator: "user".into(),
            action: action.into(),
            target_type: Some("meeting".into()),
            target_id: Some("m-1".into()),
            detail,
            result: "success".into(),
            prev_hash: prev.map(str::to_string),
            row_hash,
        }
    }

    fn chain() -> Vec<AuditRow> {
        let a = row(1, None, "app_start");
        let b = row(2, Some(&a.row_hash.clone()), "export_docx");
        let c = row(3, Some(&b.row_hash.clone()), "app_exit");
        vec![a, b, c]
    }

    #[test]
    fn hash_matches_the_python_implementation() {
        // hashlib.sha256("|".join(["0"*64, "2026-10-18T08:00:00Z", "张三", "export", "meeting",
        //   "m-1", '{"format":"docx","中文":1}', "success"]).encode()).hexdigest()
        let detail = detail_to_json(&json!({"中文": 1, "format": "docx"}));
        assert_eq!(detail, r#"{"format":"docx","中文":1}"#);
        let hash = compute_row_hash(None, "2026-10-18T08:00:00Z", "张三", "export", Some("meeting"), Some("m-1"), &detail, "success");
        assert_eq!(hash, "54bde97d12d29fee019cf3d0aad0fb20626f38610eea25084022b11e9b593a73");
    }

    #[test]
    fn intact_chain_verifies() {
        assert!(verify_rows(&chain()).is_empty());
    }

    #[test]
    fn edited_row_is_reported_once() {
        let mut rows = chain();
        rows[1].action = "app_start".into();
        let problems = verify_rows(&rows);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].log_id, 2);
        assert_eq!(problems[0].kind, "row_hash_mismatch");
    }

    #[test]
    fn deleted_row_breaks_the_link() {
        let mut rows = chain();
        rows.remove(1);
        let problems = verify_rows(&rows);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].kind, "prev_hash_mismatch");
    }

    #[test]
    fn content_is_refused_in_detail() {
        assert!(check_detail_safety(&json!({"meeting_id": "m-1", "segments": 42})).is_ok());
        assert!(check_detail_safety(&json!({"text": "x"})).is_err());
        assert!(check_detail_safety(&json!({"Title": "x"})).is_err());
        assert!(check_detail_safety(&json!({"segments": [{"a": 1}]})).is_err());
        assert!(check_detail_safety(&json!({"note": "字".repeat(300)})).is_err());
    }

    #[test]
    fn every_check_item_action_is_registered() {
        for (_, actions) in CHECK_ITEMS {
            for action in *actions {
                assert!(action_label(action).is_some(), "{action} is not in ACTIONS");
            }
        }
    }
}
