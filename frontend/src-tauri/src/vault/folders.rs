//! Meeting folders: the recording and transcripts.json are encrypted once a recording, import
//! or retranscription is finished, and decrypted to a temporary file when they are needed again.
//! metadata.json (name, times, devices, language) stays readable.

use super::file_crypt;
use anyhow::{anyhow, Result};
use log::{info, warn};
use std::path::{Path, PathBuf};

/// Extensions of recordings and imported audio that get encrypted
const AUDIO_EXTENSIONS: &[&str] = &["mp4", "m4a", "wav", "mp3", "flac", "ogg", "mkv", "webm", "wma", "aac"];
const TRANSCRIPTS_FILE: &str = "transcripts.json";

fn should_seal(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name == TRANSCRIPTS_FILE {
        return true;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
}

/// Encrypts the recording and transcripts.json in a meeting folder, leaving other files alone.
/// Already encrypted files are skipped. Returns how many files were encrypted.
pub fn seal_folder(folder: &Path) -> Result<usize> {
    let key = super::key().ok_or_else(|| anyhow!("数据密钥没有加载"))?;
    let mut sealed = 0;
    for entry in std::fs::read_dir(folder)?.flatten() {
        let path = entry.path();
        if !path.is_file() || !should_seal(&path) || file_crypt::is_encrypted(&path) {
            continue;
        }
        file_crypt::encrypt_in_place(key, &path)?;
        sealed += 1;
    }
    if sealed > 0 {
        info!("Encrypted {} file(s) in {}", sealed, folder.display());
    }
    Ok(sealed)
}

/// `seal_folder`, logging instead of failing: the data is still in place, only unencrypted
pub fn seal_folder_or_warn(folder: &Path) {
    if let Err(e) = seal_folder(folder) {
        warn!("Could not encrypt files in {}: {}", folder.display(), e);
    }
}

/// An encrypted recording decrypted for reading; the temporary copy goes away when dropped
pub struct PlainCopy {
    path: PathBuf,
    _dir: Option<tempfile::TempDir>,
}

impl PlainCopy {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// A readable version of `path`: itself when it is not encrypted, else a temporary decrypted
/// copy with the original file name (decoders go by the extension)
pub fn readable(path: &Path) -> Result<PlainCopy> {
    if !file_crypt::is_encrypted(path) {
        return Ok(PlainCopy { path: path.to_path_buf(), _dir: None });
    }
    let key = super::key().ok_or_else(|| anyhow!("数据密钥没有加载"))?;
    let name = file_crypt::plain_path(path)
        .and_then(|p| p.file_name().map(|n| n.to_os_string()))
        .unwrap_or_else(|| "audio".into());
    let dir = tempfile::Builder::new().prefix("meeting-audio-").tempdir()?;
    let plain = dir.path().join(name);
    file_crypt::decrypt_to(key, path, &plain)?;
    Ok(PlainCopy { path: plain, _dir: Some(dir) })
}

/// The encrypted form of a recording the folder lookup did not find in the clear
pub fn find_encrypted(folder: &Path, plain_names: &[&str]) -> Option<PathBuf> {
    for name in plain_names {
        let path = file_crypt::encrypted_path(&folder.join(name));
        if path.exists() {
            return Some(path);
        }
    }
    std::fs::read_dir(folder).ok()?.flatten().map(|e| e.path()).find(|path| {
        file_crypt::plain_path(path).is_some_and(|plain| {
            plain.file_name().and_then(|n| n.to_str()) != Some(TRANSCRIPTS_FILE) && should_seal(&plain)
        })
    })
}

/// Encrypts meeting folders made before encryption existed. Skips folders while an import or
/// retranscription might be reading their audio.
pub async fn seal_existing_folders(pool: &sqlx::SqlitePool) {
    let folders: Vec<String> = match sqlx::query_scalar("SELECT folder_path FROM meetings WHERE folder_path IS NOT NULL")
        .fetch_all(pool)
        .await
    {
        Ok(folders) => folders,
        Err(e) => {
            warn!("Could not list meeting folders to encrypt: {}", e);
            return;
        }
    };
    let result = tokio::task::spawn_blocking(move || {
        let mut sealed = 0;
        for folder in folders {
            if crate::audio::import::is_import_in_progress() || crate::audio::retranscription::is_retranscription_in_progress() {
                info!("Stopping the folder encryption pass while audio is being processed; it continues next start");
                break;
            }
            let folder = PathBuf::from(folder);
            if folder.is_dir() {
                match seal_folder(&folder) {
                    Ok(n) => sealed += n,
                    Err(e) => warn!("Could not encrypt files in {}: {}", folder.display(), e),
                }
            }
        }
        sealed
    })
    .await;
    if let Ok(sealed) = result {
        if sealed > 0 {
            info!("Encrypted {} existing recording/transcript file(s)", sealed);
            crate::audit::record("data_encrypt", Some("setting"), Some("meeting_files"), serde_json::json!({ "files": sealed })).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recordings_and_transcripts_are_sealed_but_metadata_is_not() {
        assert!(should_seal(Path::new("/m/audio.mp4")));
        assert!(should_seal(Path::new("/m/会议录音.WAV")));
        assert!(should_seal(Path::new("/m/transcripts.json")));
        assert!(!should_seal(Path::new("/m/metadata.json")));
        assert!(!should_seal(Path::new("/m/audio.mp4.enc")));
    }
}
