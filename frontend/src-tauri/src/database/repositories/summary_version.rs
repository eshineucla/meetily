use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;

/// One saved version of a meeting's summary
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SummaryVersion {
    pub version: i64,
    pub created_at: String,
    /// "generated" or "edited"
    pub source: String,
    /// The summary JSON as stored in summary_processes.result
    pub result: String,
}

pub struct SummaryVersionsRepository;

impl SummaryVersionsRepository {
    /// Save `result` as the meeting's next version and return its number
    pub async fn add(
        pool: &SqlitePool,
        meeting_id: &str,
        source: &str,
        result: &str,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            r#"
            INSERT INTO summary_versions (meeting_id, version, created_at, source, result)
            SELECT ?, COALESCE(MAX(version), 0) + 1, ?, ?, ?
            FROM summary_versions WHERE meeting_id = ?
            RETURNING version
            "#,
        )
        .bind(meeting_id)
        .bind(Utc::now().to_rfc3339())
        .bind(source)
        .bind(result)
        .bind(meeting_id)
        .fetch_one(pool)
        .await
    }

    /// All versions of a meeting's summary, newest first
    pub async fn list(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<SummaryVersion>, sqlx::Error> {
        sqlx::query_as::<_, SummaryVersion>(
            "SELECT version, created_at, source, result FROM summary_versions WHERE meeting_id = ? ORDER BY version DESC",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }

    pub async fn get(
        pool: &SqlitePool,
        meeting_id: &str,
        version: i64,
    ) -> Result<Option<SummaryVersion>, sqlx::Error> {
        sqlx::query_as::<_, SummaryVersion>(
            "SELECT version, created_at, source, result FROM summary_versions WHERE meeting_id = ? AND version = ?",
        )
        .bind(meeting_id)
        .bind(version)
        .fetch_optional(pool)
        .await
    }

    /// Make `result` the meeting's current summary. If the current summary is not
    /// already one of the saved versions (it was edited by hand), keep it as a new
    /// "edited" version first so nothing is lost.
    pub async fn restore(pool: &SqlitePool, meeting_id: &str, result: &str) -> Result<bool, sqlx::Error> {
        let current: Option<Option<String>> =
            sqlx::query_scalar("SELECT result FROM summary_processes WHERE meeting_id = ?")
                .bind(meeting_id)
                .fetch_optional(pool)
                .await?;
        let Some(current) = current else {
            return Ok(false);
        };

        if let Some(current) = current.filter(|c| c.as_str() != result) {
            let saved: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM summary_versions WHERE meeting_id = ? AND result = ?)",
            )
            .bind(meeting_id)
            .bind(&current)
            .fetch_one(pool)
            .await?;
            if !saved {
                Self::add(pool, meeting_id, "edited", &current).await?;
            }
        }

        let now = Utc::now();
        let update = sqlx::query(
            "UPDATE summary_processes SET result = ?, status = 'completed', error = NULL, updated_at = ? WHERE meeting_id = ?",
        )
        .bind(result)
        .bind(now)
        .bind(meeting_id)
        .execute(pool)
        .await?;
        sqlx::query("UPDATE meetings SET updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(meeting_id)
            .execute(pool)
            .await?;
        Ok(update.rows_affected() == 1)
    }
}
