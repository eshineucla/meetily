//! Glossary (术语库): fixes names, organisations and terms that speech recognition gets wrong.
//! Transcripts are corrected when they are produced (live recording, audio import,
//! retranscription) and when a text transcript is imported; Whisper also gets the correct
//! spellings in its prompt. The rules are in [`matcher`].
//!
//! Corrections are counted in the audit log (counts only: aliases and names are not written there).

pub mod commands;
pub mod csv_io;
pub mod matcher;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use once_cell::sync::Lazy;
use sqlx::{Row, SqlitePool};

use matcher::{AliasIndex, CorrectionHit, GlossaryEntry};

/// Whisper reads at most about 224 prompt tokens, and a Chinese character can take two or three
/// of them; this leaves room for the instruction the prompt starts with.
pub const PROMPT_BUDGET_CHARS: usize = 60;

struct Compiled {
    index: AliasIndex,
    prompt_terms: Vec<String>,
}

static POOL: RwLock<Option<SqlitePool>> = RwLock::new(None);
static COMPILED: Lazy<RwLock<Arc<Compiled>>> = Lazy::new(|| {
    RwLock::new(Arc::new(Compiled { index: AliasIndex::new(), prompt_terms: Vec::new() }))
});
/// Corrections made to live transcript lines since the recording started
static LIVE_HITS: AtomicUsize = AtomicUsize::new(0);

/// Called whenever the database is opened and migrated
pub fn attach_database(pool: SqlitePool) {
    *POOL.write().unwrap() = Some(pool);
    tauri::async_runtime::spawn(async {
        if let Err(e) = reload().await {
            log::error!("Glossary not loaded: {}", e);
        }
    });
}

fn pool() -> Result<SqlitePool, String> {
    POOL.read().unwrap().clone().ok_or_else(|| "数据库尚未打开".to_string())
}

pub async fn load_entries() -> Result<Vec<GlossaryEntry>, String> {
    let rows = sqlx::query(
        "SELECT id, category, canonical, aliases, note, enabled FROM glossary_entry ORDER BY category, canonical",
    )
    .fetch_all(&pool()?)
    .await
    .map_err(|e| format!("读取术语库失败：{}", e))?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let aliases: String = r.get("aliases");
            GlossaryEntry {
                id: r.get("id"),
                category: r.get("category"),
                canonical: r.get("canonical"),
                aliases: matcher::parse_aliases(&aliases).unwrap_or_default(),
                note: r.get("note"),
                enabled: r.get::<i64, _>("enabled") != 0,
            }
        })
        .collect())
}

/// Rebuilds the in-memory index after the glossary changes
pub async fn reload() -> Result<(), String> {
    let entries = load_entries().await?;
    let compiled = Compiled {
        index: matcher::build_alias_index(&entries),
        prompt_terms: matcher::prompt_terms(&entries, PROMPT_BUDGET_CHARS).0,
    };
    *COMPILED.write().unwrap() = Arc::new(compiled);
    Ok(())
}

fn compiled() -> Arc<Compiled> {
    COMPILED.read().unwrap().clone()
}

pub fn correct(text: &str) -> (String, Vec<CorrectionHit>) {
    matcher::apply_corrections(text, &compiled().index)
}

/// Corrects every text in place and returns the combined hits and how many texts changed
pub fn correct_all<'a>(texts: impl IntoIterator<Item = &'a mut String>) -> (Vec<CorrectionHit>, usize) {
    let compiled = compiled();
    let mut total = Vec::new();
    let mut changed = 0;
    for text in texts {
        let (fixed, hits) = matcher::apply_corrections(text, &compiled.index);
        if !hits.is_empty() {
            *text = fixed;
            changed += 1;
            matcher::merge_hits(&mut total, hits);
        }
    }
    (total, changed)
}

/// For a line of live transcript; final lines are counted for the recording's audit record
/// (partial lines are replaced by their final version, so counting them would count twice)
pub fn correct_live(text: String, is_final: bool) -> String {
    let (fixed, hits) = correct(&text);
    if hits.is_empty() {
        return text;
    }
    if is_final {
        LIVE_HITS.fetch_add(hits.iter().map(|h| h.count).sum(), Ordering::Relaxed);
    }
    fixed
}

/// Number of live corrections since the last call
pub fn take_live_hits() -> usize {
    LIVE_HITS.swap(0, Ordering::Relaxed)
}

/// The Whisper prompt: the base instruction followed by glossary spellings, if any
pub fn whisper_prompt(base: &str) -> String {
    let terms = &compiled().prompt_terms;
    if terms.is_empty() {
        base.to_string()
    } else {
        format!("{}会议涉及：{}。", base, terms.join("、"))
    }
}

/// One audit record for the corrections made to a meeting's transcript (counts only)
pub async fn record_corrections(meeting_id: &str, hits: &[CorrectionHit], segments_changed: usize) {
    if hits.is_empty() {
        return;
    }
    crate::audit::record_meeting(
        "glossary_correct",
        meeting_id,
        serde_json::json!({
            "hits": hits.iter().map(|h| h.count).sum::<usize>(),
            "entries": hits.len(),
            "segments_changed": segments_changed,
        }),
    )
    .await;
}
