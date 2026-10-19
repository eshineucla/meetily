use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{ConnectOptions, Connection, Result, Sqlite, SqlitePool, Transaction};
use std::fs;
use std::path::Path;
use tauri::Manager;

#[derive(Clone)]
pub struct DatabaseManager {
    pool: SqlitePool,
}

impl DatabaseManager {
    pub async fn new(tauri_db_path: &str, backend_db_path: &str) -> Result<Self> {
        if let Some(parent_dir) = Path::new(tauri_db_path).parent() {
            if !parent_dir.exists() {
                fs::create_dir_all(parent_dir).map_err(|e| sqlx::Error::Io(e))?;
            }
        }

        if !Path::new(tauri_db_path).exists() && Path::new(backend_db_path).exists() {
            log::info!(
                "Copying database from {} to {}",
                backend_db_path,
                tauri_db_path
            );
            fs::copy(backend_db_path, tauri_db_path).map_err(|e| sqlx::Error::Io(e))?;
        }

        let key = crate::vault::sqlcipher_key()
            .ok_or_else(|| sqlx::Error::Configuration("数据密钥没有加载".into()))?;
        let encrypted_now = crate::vault::database_is_plain(Path::new(tauri_db_path));
        if encrypted_now {
            Self::encrypt_plain_database(Path::new(tauri_db_path), &key).await?;
        }

        let options = SqliteConnectOptions::new()
            .filename(tauri_db_path)
            .create_if_missing(true)
            .pragma("key", key);
        let pool = SqlitePoolOptions::new().connect_with(options).await?;

        sqlx::migrate!("./migrations").run(&pool).await?;
        crate::audit::attach_database(pool.clone());
        crate::glossary::attach_database(pool.clone());
        if encrypted_now {
            crate::audit::record("data_encrypt", Some("setting"), Some("database"), serde_json::json!({})).await;
        }
        // Recordings and transcript files from before encryption, in the background
        let sweep_pool = pool.clone();
        tauri::async_runtime::spawn(async move { crate::vault::folders::seal_existing_folders(&sweep_pool).await });

        Ok(DatabaseManager { pool })
    }

    /// Turns a plain SQLite database into an encrypted one in place: SQLCipher exports it into a
    /// new encrypted file, the copy is checked against the original, then replaces it.
    async fn encrypt_plain_database(path: &Path, key: &str) -> Result<()> {
        log::info!("Encrypting the existing meeting database");
        let encrypted = path.with_extension("sqlite.encrypting");
        let _ = fs::remove_file(&encrypted);

        // ATTACH opens the new file with this connection's flags, so it must be allowed to create it
        let mut plain = SqliteConnectOptions::new().filename(path).create_if_missing(true).connect().await?;
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&mut plain).await?;
        let attach = format!(
            "ATTACH DATABASE '{}' AS encrypted KEY {}",
            encrypted.to_string_lossy().replace('\'', "''"),
            key
        );
        sqlx::query(&attach).execute(&mut plain).await?;
        sqlx::query("SELECT sqlcipher_export('encrypted')").fetch_all(&mut plain).await?;
        sqlx::query("DETACH DATABASE encrypted").execute(&mut plain).await?;

        // Every table must have come across with all its rows
        let tables: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
                .fetch_all(&mut plain)
                .await?;
        let mut counts = Vec::new();
        for table in &tables {
            let sql = format!("SELECT COUNT(*) FROM \"{}\"", table.replace('"', "\"\""));
            counts.push(sqlx::query_scalar::<_, i64>(&sql).fetch_one(&mut plain).await?);
        }
        plain.close().await?;

        let mut check = SqliteConnectOptions::new().filename(&encrypted).pragma("key", key.to_string()).connect().await?;
        for (table, expected) in tables.iter().zip(&counts) {
            let sql = format!("SELECT COUNT(*) FROM \"{}\"", table.replace('"', "\"\""));
            let found: i64 = sqlx::query_scalar(&sql).fetch_one(&mut check).await?;
            if found != *expected {
                check.close().await?;
                let _ = fs::remove_file(&encrypted);
                return Err(sqlx::Error::Protocol(format!("加密后的数据库核对失败（{} 表）", table)));
            }
        }
        check.close().await?;

        // The plain WAL is empty after the checkpoint; it must not be applied to the encrypted file
        fs::rename(&encrypted, path).map_err(sqlx::Error::Io)?;
        for suffix in ["sqlite-wal", "sqlite-shm"] {
            let _ = fs::remove_file(path.with_extension(suffix));
        }
        log::info!("Meeting database encrypted ({} tables)", tables.len());
        Ok(())
    }

    // NOTE: So for the first time users they needs to start the application
    // after they can just delete the existing .sqlite file and then copy the existing .db file to
    // the current app dir, So the system detects legacy db and copy it and starts with that data
    // (Newly created .sqlite with the copied content from .db)
    pub async fn new_from_app_handle(app_handle: &tauri::AppHandle) -> Result<Self> {
        // Resolve the app's data directory
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .expect("failed to get app data dir");
        if !app_data_dir.exists() {
            fs::create_dir_all(&app_data_dir).map_err(|e| sqlx::Error::Io(e))?;
        }

        // Define database paths
        let tauri_db_path = app_data_dir
            .join("meeting_minutes.sqlite")
            .to_string_lossy()
            .to_string();
        // Legacy backend DB path (for auto-migration if exists)
        let backend_db_path = app_data_dir
            .join("meeting_minutes.db")
            .to_string_lossy()
            .to_string();

        // WAL file paths for defensive cleanup
        let wal_path = app_data_dir.join("meeting_minutes.sqlite-wal");
        let shm_path = app_data_dir.join("meeting_minutes.sqlite-shm");

        crate::vault::init(&app_data_dir, Path::new(&tauri_db_path))
            .map_err(|e| sqlx::Error::Configuration(e.into()))?;

        log::info!("Tauri DB path: {}", tauri_db_path);
        log::info!("Legacy backend DB path: {}", backend_db_path);

        // Try to open database with defensive WAL handling
        match Self::new(&tauri_db_path, &backend_db_path).await {
            Ok(db_manager) => {
                log::info!("Database opened successfully");
                Ok(db_manager)
            }
            Err(e) => {
                // Check if error is due to corrupted WAL file
                let error_msg = e.to_string();
                if error_msg.contains("malformed") || error_msg.contains("corrupt") {
                    log::warn!("Database appears corrupted, likely due to orphaned WAL file. Attempting recovery...");
                    log::warn!("Error details: {}", error_msg);

                    // Delete potentially corrupted WAL/SHM files
                    if wal_path.exists() {
                        match fs::remove_file(&wal_path) {
                            Ok(_) => log::info!("Removed orphaned WAL file: {:?}", wal_path),
                            Err(e) => log::warn!("Failed to remove WAL file: {}", e),
                        }
                    }
                    if shm_path.exists() {
                        match fs::remove_file(&shm_path) {
                            Ok(_) => log::info!("Removed orphaned SHM file: {:?}", shm_path),
                            Err(e) => log::warn!("Failed to remove SHM file: {}", e),
                        }
                    }

                    // Retry connection without WAL files
                    log::info!("Retrying database connection after WAL cleanup...");
                    match Self::new(&tauri_db_path, &backend_db_path).await {
                        Ok(db_manager) => {
                            log::info!("Database opened successfully after WAL recovery");
                            Ok(db_manager)
                        }
                        Err(retry_err) => {
                            log::error!("Database connection failed even after WAL cleanup: {}", retry_err);
                            Err(retry_err)
                        }
                    }
                } else {
                    // Not a WAL-related error, propagate original error
                    log::error!("Database connection failed: {}", error_msg);
                    Err(e)
                }
            }
        }
    }

    /// Check if this is the first launch (sqlite database doesn't exist yet)
    pub async fn is_first_launch(app_handle: &tauri::AppHandle) -> Result<bool> {
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .expect("failed to get app data dir");

        let tauri_db_path = app_data_dir.join("meeting_minutes.sqlite");

        Ok(!tauri_db_path.exists())
    }

    /// Import a legacy database from the specified path and initialize
    pub async fn import_legacy_database(
        app_handle: &tauri::AppHandle,
        legacy_db_path: &str,
    ) -> Result<Self> {
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .expect("failed to get app data dir");

        if !app_data_dir.exists() {
            fs::create_dir_all(&app_data_dir).map_err(|e| sqlx::Error::Io(e))?;
        }

        // Copy legacy database to app data directory as meeting_minutes.db
        let target_legacy_path = app_data_dir.join("meeting_minutes.db");
        log::info!(
            "Copying legacy database from {} to {}",
            legacy_db_path,
            target_legacy_path.display()
        );

        fs::copy(legacy_db_path, &target_legacy_path).map_err(|e| sqlx::Error::Io(e))?;

        // Now use the standard initialization which will detect and migrate the legacy db
        Self::new_from_app_handle(app_handle).await
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn with_transaction<T, F, Fut>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Transaction<'_, Sqlite>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut tx = self.pool.begin().await?;
        let result = f(&mut tx).await;

        match result {
            Ok(val) => {
                tx.commit().await?;
                Ok(val)
            }
            Err(err) => {
                tx.rollback().await?;
                Err(err)
            }
        }
    }

    /// Cleanup database connection and checkpoint WAL
    /// This should be called on application shutdown to ensure:
    /// - All WAL changes are written to the main database file
    /// - The .wal and .shm files are deleted
    /// - Connection pool is gracefully closed
    pub async fn cleanup(&self) -> Result<()> {
        log::info!("Starting database cleanup...");

        // Force checkpoint of WAL to main database file and remove WAL file
        // TRUNCATE mode: checkpoints all pages AND deletes the WAL file
        match sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await
        {
            Ok(_) => log::info!("WAL checkpoint completed successfully"),
            Err(e) => log::warn!("WAL checkpoint failed (non-fatal): {}", e),
        }

        // Close the connection pool gracefully
        self.pool.close().await;
        log::info!("Database connection pool closed");

        Ok(())
    }
}
