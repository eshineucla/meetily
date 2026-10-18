//! Encryption of meeting data at rest.
//!
//! One random 256-bit data key protects everything: it is the SQLCipher key of the meeting
//! database (transcripts, minutes, versions, background notes, audit log), and it encrypts the
//! recordings and transcript files in meeting folders (`file_crypt`). The key lives in the
//! operating system's credential store (Windows Credential Manager, macOS keychain). Where that
//! is unavailable it is kept in `data.key` in the app data folder, and the app says so instead
//! of presenting that as protected.
//!
//! The key is never replaced while encrypted data exists: if it cannot be found the app stops
//! with instructions to restore the backup, rather than starting over with a new key.

pub mod commands;
pub mod file_crypt;
pub mod folders;

use log::{info, warn};
use rand::RngCore;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Also the name a key backup is restored under
pub const KEY_FILE: &str = "data.key";
#[cfg(any(windows, target_os = "macos"))]
const SERVICE: &str = "MeetingMinutes";
#[cfg(any(windows, target_os = "macos"))]
const ACCOUNT: &str = "data-key";

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyStorage {
    /// Windows Credential Manager or the macOS keychain
    CredentialStore,
    /// `data.key` next to the database
    File,
}

struct DataKey {
    bytes: [u8; 32],
    storage: KeyStorage,
    app_data_dir: PathBuf,
}

static KEY: OnceLock<DataKey> = OnceLock::new();

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn from_hex(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(key)
}

/// The key in a key file or backup: the first line that is not empty or a `#` comment
fn parse_key_file(text: &str) -> Option<[u8; 32]> {
    text.lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with('#')).and_then(from_hex)
}

/// Ok(None) when the store has no key; Err when the store itself is unavailable
#[cfg(any(windows, target_os = "macos"))]
fn store_get() -> Result<Option<[u8; 32]>, String> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(text) => from_hex(&text).map(Some).ok_or_else(|| "credential store holds a malformed key".to_string()),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn store_get() -> Result<Option<[u8; 32]>, String> {
    Err("no credential store on this platform".to_string())
}

/// Saves the key and reads it back, so a store that only pretends to keep it is not trusted
#[cfg(any(windows, target_os = "macos"))]
fn store_set(key: &[u8; 32]) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())?;
    entry.set_password(&to_hex(key)).map_err(|e| e.to_string())?;
    match store_get()? {
        Some(read) if &read == key => Ok(()),
        _ => Err("credential store did not keep the key".to_string()),
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn store_set(_key: &[u8; 32]) -> Result<(), String> {
    Err("no credential store on this platform".to_string())
}

/// Whether the database file holds data that is not plain SQLite (so it is encrypted)
fn database_is_encrypted(db_path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 16];
    match std::fs::File::open(db_path).and_then(|mut f| f.read_exact(&mut head)) {
        Ok(()) => &head != b"SQLite format 3\0",
        Err(_) => false,
    }
}

/// Whether the database file is a plain (unencrypted) SQLite database
pub fn database_is_plain(db_path: &Path) -> bool {
    db_path.exists() && std::fs::metadata(db_path).map(|m| m.len() > 0).unwrap_or(false) && !database_is_encrypted(db_path)
}

fn missing_key_message(app_data_dir: &Path) -> String {
    format!(
        "找不到会议数据的密钥，已加密的会议数据无法打开。请把备份的密钥文件改名为 {}，放到 {} 文件夹里，再重新启动程序。",
        KEY_FILE,
        app_data_dir.display()
    )
}

/// Loads the data key, creating it on first use. Must run before the database is opened.
pub fn init(app_data_dir: &Path, db_path: &Path) -> Result<(), String> {
    if KEY.get().is_some() {
        return Ok(());
    }
    let key_file = app_data_dir.join(KEY_FILE);
    let file_key = std::fs::read_to_string(&key_file).ok().and_then(|text| parse_key_file(&text));

    let (bytes, storage) = match store_get() {
        Ok(Some(key)) => {
            if file_key.is_some_and(|file| file == key) {
                // A restored backup that the store already holds
                let _ = std::fs::remove_file(&key_file);
            } else if file_key.is_some() {
                warn!("{} differs from the key in the credential store; using the stored key", KEY_FILE);
            }
            (key, KeyStorage::CredentialStore)
        }
        store => {
            if let Err(e) = &store {
                warn!("Credential store unavailable: {}", e);
            }
            match file_key {
                Some(key) => match store_set(&key) {
                    Ok(()) => {
                        info!("Moved the data key from {} into the credential store", KEY_FILE);
                        let _ = std::fs::remove_file(&key_file);
                        (key, KeyStorage::CredentialStore)
                    }
                    Err(_) => (key, KeyStorage::File),
                },
                None if database_is_encrypted(db_path) => return Err(missing_key_message(app_data_dir)),
                None => {
                    let mut key = [0u8; 32];
                    rand::thread_rng().fill_bytes(&mut key);
                    match store_set(&key) {
                        Ok(()) => {
                            info!("Created a data key in the credential store");
                            (key, KeyStorage::CredentialStore)
                        }
                        Err(e) => {
                            warn!("Keeping the data key in {} because the credential store failed: {}", KEY_FILE, e);
                            std::fs::write(&key_file, format!("# 会议纪要数据密钥，删除后已加密的会议数据将无法打开\n{}\n", to_hex(&key)))
                                .map_err(|e| format!("保存数据密钥失败：{}", e))?;
                            (key, KeyStorage::File)
                        }
                    }
                }
            }
        }
    };

    let _ = KEY.set(DataKey { bytes, storage, app_data_dir: app_data_dir.to_path_buf() });
    Ok(())
}

/// The data key, once `init` has run
pub fn key() -> Option<&'static [u8; 32]> {
    KEY.get().map(|k| &k.bytes)
}

/// Value for SQLCipher's `PRAGMA key`: the raw key, so no password derivation is applied
pub fn sqlcipher_key() -> Option<String> {
    key().map(|k| format!("\"x'{}'\"", to_hex(k)))
}

pub fn storage() -> Option<KeyStorage> {
    KEY.get().map(|k| k.storage)
}

fn app_data_dir() -> Option<&'static Path> {
    KEY.get().map(|k| k.app_data_dir.as_path())
}

/// Text of a key backup file
fn backup_text(key: &[u8; 32], app_data_dir: &Path) -> String {
    format!(
        "# 会议纪要数据密钥备份（{}）\n\
         # 会议数据库、录音和记录稿文件都用这个密钥加密。重装系统、换 Windows 账户或凭据丢失后，\n\
         # 把本文件改名为 {}，放到 {} 文件夹里，再启动程序，即可恢复。\n\
         # 拿到本文件的人可以解开会议数据：请单独保管，不要和电脑放在一起。\n\
         {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M"),
        KEY_FILE,
        app_data_dir.display(),
        to_hex(key)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let key: [u8; 32] = std::array::from_fn(|i| (i * 9) as u8);
        assert_eq!(from_hex(&to_hex(&key)), Some(key));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex(&"zz".repeat(32)), None);
    }

    #[test]
    fn key_files_and_backups_parse() {
        let key = [0xabu8; 32];
        let backup = backup_text(&key, Path::new("C:/data"));
        assert_eq!(parse_key_file(&backup), Some(key));
        assert_eq!(parse_key_file(&format!("\n  {}  \n", to_hex(&key))), Some(key));
        assert_eq!(parse_key_file("# only a comment\n"), None);
    }

    #[test]
    fn database_kind_is_read_from_the_header() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain.sqlite");
        std::fs::write(&plain, b"SQLite format 3\0rest of the page").unwrap();
        let encrypted = dir.path().join("enc.sqlite");
        std::fs::write(&encrypted, [0x5au8; 64]).unwrap();
        let empty = dir.path().join("empty.sqlite");
        std::fs::write(&empty, b"").unwrap();
        assert!(database_is_plain(&plain) && !database_is_encrypted(&plain));
        assert!(!database_is_plain(&encrypted) && database_is_encrypted(&encrypted));
        assert!(!database_is_plain(&empty) && !database_is_encrypted(&empty));
        assert!(!database_is_plain(&dir.path().join("missing.sqlite")));
    }
}
