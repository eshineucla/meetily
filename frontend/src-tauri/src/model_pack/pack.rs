//! Model packs: a plain zip of model files laid out as in the app's models directory, so a
//! machine without internet access (the classified build, or a network that cannot reach the
//! model hosts) can install models that were downloaded elsewhere.
//!
//! ```text
//! ggml-large-v3-turbo.bin                     Whisper
//! parakeet/sense-voice-small-int8/model.int8.onnx
//! parakeet/sense-voice-small-int8/tokens.txt  ONNX engine (SenseVoice / Parakeet)
//! summary/Qwen3.5-2B-Q4_K_M.gguf              built-in summary model
//! ```
//!
//! Only files named in the app's model catalogs are written, and only with a plausible size;
//! anything else in the zip is ignored. A leading `models/` folder is accepted. This module has
//! no Tauri dependency so that it can be tested on its own.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

const MIB: u64 = 1024 * 1024;
const COPY_BUFFER: usize = 8 * 1024 * 1024;

/// One file the app knows how to use
#[derive(Debug, Clone)]
pub struct CatalogFile {
    /// Path relative to the models directory, with `/` separators
    pub rel_path: String,
    /// Model this file belongs to, e.g. "SenseVoice 中文 (sense-voice-small-int8)"
    pub model_id: String,
    pub model_label: String,
    pub kind: ModelKind,
    pub min_bytes: u64,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Whisper,
    Onnx,
    Summary,
}

impl CatalogFile {
    pub fn accepts_size(&self, bytes: u64) -> bool {
        bytes >= self.min_bytes && bytes <= self.max_bytes
    }
}

/// Whisper: `(name, filename)` pairs. Any file of at least 1 MiB is accepted, as in the engine.
pub fn whisper_files(catalog: &[(&str, &str)]) -> Vec<CatalogFile> {
    catalog
        .iter()
        .map(|(name, filename)| CatalogFile {
            rel_path: filename.to_string(),
            model_id: format!("whisper:{}", name),
            model_label: format!("Whisper {}", name),
            kind: ModelKind::Whisper,
            min_bytes: MIB,
            max_bytes: u64::MAX,
        })
        .collect()
}

/// ONNX engine: `(model name, file name, exact size)`
pub fn onnx_files(catalog: &[(&str, &str, u64)]) -> Vec<CatalogFile> {
    catalog
        .iter()
        .map(|(model, filename, bytes)| CatalogFile {
            rel_path: format!("parakeet/{}/{}", model, filename),
            model_id: format!("onnx:{}", model),
            model_label: onnx_label(model),
            kind: ModelKind::Onnx,
            min_bytes: *bytes,
            max_bytes: *bytes,
        })
        .collect()
}

fn onnx_label(model: &str) -> String {
    if model.starts_with("sense-voice") {
        format!("SenseVoice 中文（{}）", model)
    } else {
        format!("Parakeet 英文（{}）", model)
    }
}

/// Built-in summary models: `(name, display name, gguf file, size in MiB)`. Sizes within 10% are
/// accepted, as in the summary model manager.
pub fn summary_files(catalog: &[(String, String, String, u64)]) -> Vec<CatalogFile> {
    catalog
        .iter()
        .map(|(name, display, gguf, size_mb)| CatalogFile {
            rel_path: format!("summary/{}", gguf),
            model_id: format!("summary:{}", name),
            model_label: format!("纪要模型 {}", display),
            kind: ModelKind::Summary,
            min_bytes: (*size_mb as f64 * 0.9) as u64 * MIB,
            max_bytes: ((*size_mb as f64 * 1.1) as u64 + 1) * MIB,
        })
        .collect()
}

/// Maps a zip entry name to a catalog file. Returns None for anything not in the catalog,
/// including absolute paths and `..` components.
pub fn match_entry<'a>(entry_name: &str, catalog: &'a [CatalogFile]) -> Option<&'a CatalogFile> {
    let normalized = entry_name.replace('\\', "/");
    let path = Path::new(&normalized);
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?.to_string()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if parts.first().map(|p| p.eq_ignore_ascii_case("models")) == Some(true) && parts.len() > 1 {
        parts.remove(0);
    }
    let rel = parts.join("/");
    catalog.iter().find(|file| file.rel_path == rel)
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub current: String,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct ImportReport {
    /// Models with at least one file written
    pub imported: Vec<String>,
    /// Models whose files were already present with the same size
    pub already_present: Vec<String>,
    /// Entries that are not model files of this app
    pub ignored: Vec<String>,
    /// Files refused because of their size, with the reason
    pub rejected: Vec<String>,
}

fn push_unique(list: &mut Vec<String>, value: &str) {
    if !list.iter().any(|v| v == value) {
        list.push(value.to_string());
    }
}

/// Extracts the model files of `pack` into `models_dir`
pub fn import_pack(
    pack: &Path,
    models_dir: &Path,
    catalog: &[CatalogFile],
    mut progress: impl FnMut(Progress),
) -> Result<ImportReport, String> {
    let file = File::open(pack).map_err(|e| format!("无法打开模型包：{}", e))?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file))
        .map_err(|e| format!("不是有效的模型包（zip）：{}", e))?;

    // First pass: decide what to extract, so progress has a total
    let mut report = ImportReport::default();
    let mut plan: Vec<(usize, &CatalogFile, u64)> = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(|e| format!("读取模型包失败：{}", e))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let size = entry.size();
        let Some(target) = match_entry(&name, catalog) else {
            if !name.ends_with('/') && !name.starts_with("__MACOSX") {
                report.ignored.push(name);
            }
            continue;
        };
        if !target.accepts_size(size) {
            report.rejected.push(format!("{}：大小 {} 字节与该模型不符", name, size));
            continue;
        }
        let destination = models_dir.join(&target.rel_path);
        if std::fs::metadata(&destination).map(|m| m.len() == size).unwrap_or(false) {
            push_unique(&mut report.already_present, &target.model_label);
            continue;
        }
        plan.push((index, target, size));
    }

    let total_bytes: u64 = plan.iter().map(|(_, _, size)| size).sum();
    let mut done_bytes = 0u64;
    let mut buffer = vec![0u8; COPY_BUFFER];
    for (index, target, size) in plan {
        let destination = models_dir.join(&target.rel_path);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建模型目录失败：{}", e))?;
        }
        let partial = partial_path(&destination);
        let result = (|| -> Result<(), String> {
            let mut entry = archive.by_index(index).map_err(|e| format!("读取模型包失败：{}", e))?;
            let mut out = BufWriter::new(File::create(&partial).map_err(|e| format!("写入模型失败：{}", e))?);
            let mut written = 0u64;
            loop {
                let n = entry.read(&mut buffer).map_err(|e| format!("模型包已损坏（{}）：{}", target.rel_path, e))?;
                if n == 0 {
                    break;
                }
                out.write_all(&buffer[..n]).map_err(|e| format!("写入模型失败：{}", e))?;
                written += n as u64;
                progress(Progress { done_bytes: done_bytes + written, total_bytes, current: target.model_label.clone() });
            }
            out.flush().map_err(|e| format!("写入模型失败：{}", e))?;
            if written != size {
                return Err(format!("模型包已损坏：{} 只读到 {} / {} 字节", target.rel_path, written, size));
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = std::fs::remove_file(&partial);
            return Err(error);
        }
        if destination.exists() {
            std::fs::remove_file(&destination)
                .map_err(|e| format!("无法替换 {}（模型可能正在使用，请先切换到其他模型）：{}", target.rel_path, e))?;
        }
        std::fs::rename(&partial, &destination).map_err(|e| format!("写入模型失败：{}", e))?;
        done_bytes += size;
        push_unique(&mut report.imported, &target.model_label);
    }
    report.already_present.retain(|label| !report.imported.contains(label));
    Ok(report)
}

fn partial_path(destination: &Path) -> PathBuf {
    let mut name = destination.file_name().unwrap_or_default().to_os_string();
    name.push(".importing");
    destination.with_file_name(name)
}

/// A model that can be put into a pack: every one of its catalog files is on disk
#[derive(Debug, Clone, Serialize)]
pub struct PackCandidate {
    pub id: String,
    pub label: String,
    pub kind: ModelKind,
    pub bytes: u64,
}

pub fn installed_models(models_dir: &Path, catalog: &[CatalogFile]) -> Vec<PackCandidate> {
    let mut by_model: BTreeMap<&str, (Vec<&CatalogFile>, bool, u64)> = BTreeMap::new();
    for file in catalog {
        let slot = by_model.entry(&file.model_id).or_insert((Vec::new(), true, 0));
        slot.0.push(file);
        match std::fs::metadata(models_dir.join(&file.rel_path)) {
            Ok(meta) if meta.is_file() && file.accepts_size(meta.len()) => slot.2 += meta.len(),
            _ => slot.1 = false,
        }
    }
    let mut out: Vec<PackCandidate> = by_model
        .into_iter()
        .filter(|(_, (_, complete, _))| *complete)
        .map(|(id, (files, _, bytes))| PackCandidate {
            id: id.to_string(),
            label: files[0].model_label.clone(),
            kind: files[0].kind,
            bytes,
        })
        .collect();
    out.sort_by_key(|c| (c.kind as u8, c.label.clone()));
    out
}

/// Writes the files of the chosen models into a zip at `destination`. Files are stored
/// uncompressed: model weights do not compress, and storing keeps import fast.
pub fn export_pack(
    models_dir: &Path,
    catalog: &[CatalogFile],
    model_ids: &[String],
    destination: &Path,
    mut progress: impl FnMut(Progress),
) -> Result<u64, String> {
    let files: Vec<&CatalogFile> = catalog.iter().filter(|f| model_ids.contains(&f.model_id)).collect();
    if files.is_empty() {
        return Err("没有选择模型".to_string());
    }
    let mut total_bytes = 0u64;
    for file in &files {
        let meta = std::fs::metadata(models_dir.join(&file.rel_path))
            .map_err(|_| format!("{} 尚未下载完整", file.model_label))?;
        total_bytes += meta.len();
    }

    let partial = partial_path(destination);
    let result = (|| -> Result<(), String> {
        let out = File::create(&partial).map_err(|e| format!("无法创建模型包：{}", e))?;
        let mut zip = zip::ZipWriter::new(BufWriter::new(out));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(true);
        let mut buffer = vec![0u8; COPY_BUFFER];
        let mut done_bytes = 0u64;
        for file in &files {
            zip.start_file(file.rel_path.as_str(), options).map_err(|e| format!("写入模型包失败：{}", e))?;
            let mut input = File::open(models_dir.join(&file.rel_path)).map_err(|e| format!("读取模型失败：{}", e))?;
            loop {
                let n = input.read(&mut buffer).map_err(|e| format!("读取模型失败：{}", e))?;
                if n == 0 {
                    break;
                }
                zip.write_all(&buffer[..n]).map_err(|e| format!("写入模型包失败：{}", e))?;
                done_bytes += n as u64;
                progress(Progress { done_bytes, total_bytes, current: file.model_label.clone() });
            }
        }
        zip.finish().map_err(|e| format!("写入模型包失败：{}", e))?.flush().map_err(|e| format!("写入模型包失败：{}", e))?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    if destination.exists() {
        std::fs::remove_file(destination).map_err(|e| format!("无法覆盖 {}：{}", destination.display(), e))?;
    }
    std::fs::rename(&partial, destination).map_err(|e| format!("无法保存模型包：{}", e))?;
    Ok(total_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Vec<CatalogFile> {
        let mut files = whisper_files(&[("tiny", "ggml-tiny.bin")]);
        files.extend(onnx_files(&[("sense-voice-small-int8", "model.int8.onnx", 10), ("sense-voice-small-int8", "tokens.txt", 4)]));
        files.extend(summary_files(&[("qwen3.5:2b".into(), "Qwen 3.5 2B".into(), "q.gguf".into(), 2)]));
        files
    }

    #[test]
    fn entry_names_map_only_to_catalog_files() {
        let catalog = catalog();
        assert!(match_entry("ggml-tiny.bin", &catalog).is_some());
        assert!(match_entry("models/ggml-tiny.bin", &catalog).is_some());
        assert!(match_entry("models\\parakeet\\sense-voice-small-int8\\tokens.txt", &catalog).is_some());
        assert!(match_entry("./summary/q.gguf", &catalog).is_some());
        assert!(match_entry("../ggml-tiny.bin", &catalog).is_none());
        assert!(match_entry("/ggml-tiny.bin", &catalog).is_none());
        assert!(match_entry("parakeet/../ggml-tiny.bin", &catalog).is_none());
        assert!(match_entry("evil.exe", &catalog).is_none());
        assert!(match_entry("summary/other.gguf", &catalog).is_none());
    }

    #[test]
    fn export_then_import_round_trip() {
        let root = std::env::temp_dir().join(format!("model-pack-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let source = root.join("source");
        let target = root.join("target");
        std::fs::create_dir_all(source.join("parakeet/sense-voice-small-int8")).unwrap();
        std::fs::create_dir_all(source.join("summary")).unwrap();
        std::fs::write(source.join("parakeet/sense-voice-small-int8/model.int8.onnx"), b"0123456789").unwrap();
        std::fs::write(source.join("parakeet/sense-voice-small-int8/tokens.txt"), b"abcd").unwrap();
        std::fs::write(source.join("summary/q.gguf"), vec![7u8; 2 * MIB as usize]).unwrap();
        std::fs::write(source.join("ggml-tiny.bin"), b"too small").unwrap();
        let catalog = catalog();

        let installed = installed_models(&source, &catalog);
        let ids: Vec<String> = installed.iter().map(|c| c.id.clone()).collect();
        assert_eq!(ids, vec!["onnx:sense-voice-small-int8".to_string(), "summary:qwen3.5:2b".to_string()]);

        let pack = root.join("pack.zip");
        let mut last = None;
        export_pack(&source, &catalog, &ids, &pack, |p| last = Some(p)).unwrap();
        assert_eq!(last.unwrap().done_bytes, 14 + 2 * MIB);

        let report = import_pack(&pack, &target, &catalog, |_| {}).unwrap();
        assert_eq!(report.imported.len(), 2);
        assert_eq!(std::fs::read(target.join("parakeet/sense-voice-small-int8/tokens.txt")).unwrap(), b"abcd");
        assert_eq!(installed_models(&target, &catalog).len(), 2);

        let again = import_pack(&pack, &target, &catalog, |_| {}).unwrap();
        assert!(again.imported.is_empty());
        assert_eq!(again.already_present.len(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn foreign_and_wrong_size_entries_are_not_written() {
        let root = std::env::temp_dir().join(format!("model-pack-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let pack = root.join("bad.zip");
        {
            let mut zip = zip::ZipWriter::new(File::create(&pack).unwrap());
            let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            zip.start_file("../escape.txt", options).unwrap();
            zip.write_all(b"x").unwrap();
            zip.start_file("parakeet/sense-voice-small-int8/tokens.txt", options).unwrap();
            zip.write_all(b"wrong size").unwrap();
            zip.finish().unwrap();
        }
        let target = root.join("models");
        let report = import_pack(&pack, &target, &catalog(), |_| {}).unwrap();
        assert!(report.imported.is_empty());
        assert_eq!(report.ignored, vec!["../escape.txt".to_string()]);
        assert_eq!(report.rejected.len(), 1);
        assert!(!root.join("escape.txt").exists());
        assert!(!target.join("parakeet").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
