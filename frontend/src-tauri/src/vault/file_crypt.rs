//! Encrypted files: AES-256-GCM in 1 MiB chunks, so recordings of any length are streamed
//! rather than held in memory.
//!
//! Layout: the 8-byte magic `MMENC01\0`, an 8-byte random nonce prefix, then chunks of
//! `u32 ciphertext length (big-endian) | ciphertext with its 16-byte tag`. Chunk `i` uses the
//! nonce `prefix || i as u32 (big-endian)` and authenticates one byte saying whether it is the
//! last chunk, so a file cut short or with chunks reordered fails to decrypt.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, bail, Context, Result};
use rand::RngCore;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

pub const MAGIC: &[u8; 8] = b"MMENC01\0";
pub const SUFFIX: &str = "enc";
const CHUNK: usize = 1 << 20;
const TAG: usize = 16;

/// Whether the file starts with the encrypted-file magic
pub fn is_encrypted(path: &Path) -> bool {
    let mut head = [0u8; 8];
    File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && &head == MAGIC
}

/// `audio.mp4` → `audio.mp4.enc`
pub fn encrypted_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".");
    name.push(SUFFIX);
    path.with_file_name(name)
}

/// `audio.mp4.enc` → `audio.mp4`; None for a name without the suffix
pub fn plain_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let plain = name.strip_suffix(&format!(".{}", SUFFIX))?;
    Some(path.with_file_name(plain))
}

/// Reads up to a full chunk; fewer bytes only at the end of the input
fn read_chunk(reader: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

fn nonce(prefix: &[u8; 8], index: u32) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[..8].copy_from_slice(prefix);
    nonce[8..].copy_from_slice(&index.to_be_bytes());
    nonce
}

pub fn encrypt_stream(key: &[u8; 32], mut input: impl Read, mut output: impl Write) -> Result<()> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow!("bad key: {}", e))?;
    let mut prefix = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut prefix);
    output.write_all(MAGIC)?;
    output.write_all(&prefix)?;

    let mut current = vec![0u8; CHUNK];
    let mut next = vec![0u8; CHUNK];
    let mut current_len = read_chunk(&mut input, &mut current)?;
    let mut index: u32 = 0;
    loop {
        // A full chunk may still be the last one; look ahead to know
        let next_len = if current_len == CHUNK { read_chunk(&mut input, &mut next)? } else { 0 };
        let last = next_len == 0;
        let sealed = cipher
            .encrypt(Nonce::from_slice(&nonce(&prefix, index)), Payload { msg: &current[..current_len], aad: &[last as u8] })
            .map_err(|_| anyhow!("encryption failed"))?;
        output.write_all(&(sealed.len() as u32).to_be_bytes())?;
        output.write_all(&sealed)?;
        if last {
            break;
        }
        std::mem::swap(&mut current, &mut next);
        current_len = next_len;
        index = index.checked_add(1).ok_or_else(|| anyhow!("file too large"))?;
    }
    output.flush()?;
    Ok(())
}

pub fn decrypt_stream(key: &[u8; 32], mut input: impl Read, mut output: impl Write) -> Result<()> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow!("bad key: {}", e))?;
    let mut head = [0u8; 16];
    input.read_exact(&mut head).context("not an encrypted file")?;
    if &head[..8] != MAGIC {
        bail!("not an encrypted file");
    }
    let prefix: [u8; 8] = head[8..].try_into().unwrap();

    let mut index: u32 = 0;
    let mut sealed = Vec::with_capacity(CHUNK + TAG);
    let mut pending: Option<Vec<u8>> = None;
    loop {
        let mut len = [0u8; 4];
        let at_end = read_chunk(&mut input, &mut len)? == 0;
        if at_end {
            // The previous chunk must have been sealed as the last one
            let chunk = pending.take().ok_or_else(|| anyhow!("encrypted file is empty"))?;
            let plain = cipher
                .decrypt(Nonce::from_slice(&nonce(&prefix, index - 1)), Payload { msg: &chunk, aad: &[1] })
                .map_err(|_| anyhow!("密钥不对或文件已损坏"))?;
            output.write_all(&plain)?;
            break;
        }
        if let Some(chunk) = pending.take() {
            let plain = cipher
                .decrypt(Nonce::from_slice(&nonce(&prefix, index - 1)), Payload { msg: &chunk, aad: &[0] })
                .map_err(|_| anyhow!("密钥不对或文件已损坏"))?;
            output.write_all(&plain)?;
        }
        let len = u32::from_be_bytes(len) as usize;
        if len < TAG || len > CHUNK + TAG {
            bail!("密钥不对或文件已损坏");
        }
        sealed.resize(len, 0);
        input.read_exact(&mut sealed).context("encrypted file is cut short")?;
        pending = Some(std::mem::take(&mut sealed));
        index = index.checked_add(1).ok_or_else(|| anyhow!("file too large"))?;
    }
    output.flush()?;
    Ok(())
}

/// Writes `bytes` encrypted to `path`, through a temporary file so a crash leaves no half file
pub fn write_encrypted(key: &[u8; 32], path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension("enc-tmp");
    encrypt_stream(key, bytes, BufWriter::new(File::create(&temp)?))?;
    std::fs::rename(&temp, path)?;
    Ok(())
}

/// The file's contents, decrypted when it is encrypted
pub fn read_maybe_encrypted(key: Option<&[u8; 32]>, path: &Path) -> Result<Vec<u8>> {
    if !is_encrypted(path) {
        return Ok(std::fs::read(path)?);
    }
    let key = key.ok_or_else(|| anyhow!("数据密钥没有加载"))?;
    let mut plain = Vec::new();
    decrypt_stream(key, BufReader::new(File::open(path)?), &mut plain)?;
    Ok(plain)
}

/// Replaces `path` with `path.enc`; the plain file is removed only after the encrypted copy
/// decrypts back to the same length. Returns the encrypted path.
pub fn encrypt_in_place(key: &[u8; 32], path: &Path) -> Result<PathBuf> {
    let target = encrypted_path(path);
    let temp = target.with_extension("enc-tmp");
    let plain_len = std::fs::metadata(path)?.len();
    encrypt_stream(key, BufReader::new(File::open(path)?), BufWriter::new(File::create(&temp)?))?;

    let mut counter = CountingWriter(0);
    decrypt_stream(key, BufReader::new(File::open(&temp)?), &mut counter)?;
    if counter.0 != plain_len {
        let _ = std::fs::remove_file(&temp);
        bail!("加密后核对失败：{}", path.display());
    }
    std::fs::rename(&temp, &target)?;
    std::fs::remove_file(path)?;
    Ok(target)
}

/// Decrypts `src` into `dst`
pub fn decrypt_to(key: &[u8; 32], src: &Path, dst: &Path) -> Result<()> {
    decrypt_stream(key, BufReader::new(File::open(src)?), BufWriter::new(File::create(dst)?))
}

struct CountingWriter(u64);

impl Write for CountingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7u8; 32];

    fn round_trip(len: usize) {
        let data: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
        let mut sealed = Vec::new();
        encrypt_stream(&KEY, &data[..], &mut sealed).unwrap();
        let mut plain = Vec::new();
        decrypt_stream(&KEY, &sealed[..], &mut plain).unwrap();
        assert_eq!(plain, data, "length {}", len);
    }

    #[test]
    fn round_trips_any_length() {
        for len in [0, 1, 100, CHUNK - 1, CHUNK, CHUNK + 1, 2 * CHUNK, 2 * CHUNK + 17] {
            round_trip(len);
        }
    }

    #[test]
    fn wrong_key_truncation_and_tampering_fail() {
        let data = vec![1u8; 2 * CHUNK + 5];
        let mut sealed = Vec::new();
        encrypt_stream(&KEY, &data[..], &mut sealed).unwrap();

        assert!(decrypt_stream(&[8u8; 32], &sealed[..], &mut Vec::new()).is_err());

        // Dropping the last chunk leaves a file whose final chunk is not marked last
        let first_two = 16 + 2 * (4 + CHUNK + TAG);
        assert!(decrypt_stream(&KEY, &sealed[..first_two], &mut Vec::new()).is_err());
        assert!(decrypt_stream(&KEY, &sealed[..sealed.len() - 3], &mut Vec::new()).is_err());

        let mut flipped = sealed.clone();
        flipped[40] ^= 1;
        assert!(decrypt_stream(&KEY, &flipped[..], &mut Vec::new()).is_err());
    }

    #[test]
    fn files_are_encrypted_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("audio.mp4");
        std::fs::write(&audio, b"secret audio").unwrap();
        let sealed = encrypt_in_place(&KEY, &audio).unwrap();
        assert_eq!(sealed, dir.path().join("audio.mp4.enc"));
        assert!(!audio.exists());
        assert!(is_encrypted(&sealed));
        assert!(!std::fs::read(&sealed).unwrap().windows(6).any(|w| w == b"secret"));
        assert_eq!(read_maybe_encrypted(Some(&KEY), &sealed).unwrap(), b"secret audio");
        assert_eq!(plain_path(&sealed).unwrap(), audio);

        let restored = dir.path().join("out.mp4");
        decrypt_to(&KEY, &sealed, &restored).unwrap();
        assert_eq!(std::fs::read(&restored).unwrap(), b"secret audio");
        assert_eq!(read_maybe_encrypted(None, &restored).unwrap(), b"secret audio");
    }
}
