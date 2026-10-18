//! Turning the exported .docx into a PDF with an office suite already on the computer:
//! Microsoft Word or WPS through their automation interface on Windows, otherwise LibreOffice.
//! The app ships no converter of its own, so the PDF keeps exactly the layout the office suite
//! gives the .docx.

use log::{info, warn};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// A large document can take a while to lay out; beyond this the office suite is assumed stuck
const CONVERT_TIMEOUT: Duration = Duration::from_secs(180);

pub const NO_CONVERTER: &str =
    "没有找到可以转换 PDF 的办公软件。请安装 Microsoft Word、WPS 或 LibreOffice，或者先导出 Word 文件，再在 Word 里另存为 PDF。";

/// Opens the document read-only and exports it; tries Word, then WPS (two automation names).
/// Paths come in through environment variables, so names with Chinese characters or quotes are safe.
#[cfg(target_os = "windows")]
const OFFICE_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$src = $env:MM_PDF_SRC
$dst = $env:MM_PDF_DST
foreach ($progId in @('Word.Application', 'KWPS.Application', 'WPS.Application')) {
  try { $app = New-Object -ComObject $progId } catch { continue }
  try {
    try { $app.Visible = $false } catch {}
    try { $app.DisplayAlerts = 0 } catch {}
    $doc = $app.Documents.Open($src, $false, $true)
    try { $doc.ExportAsFixedFormat($dst, 17) } catch { $doc.SaveAs2($dst, 17) }
    $doc.Close($false)
    $app.Quit()
    Write-Output $progId
    exit 0
  } catch {
    [Console]::Error.WriteLine("${progId}: $($_.Exception.Message)")
    try { $app.Quit() } catch {}
  }
}
exit 3
"#;

fn hide_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = command;
}

async fn run(mut command: Command) -> Result<std::process::Output, String> {
    hide_window(&mut command);
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let child = command.spawn().map_err(|e| e.to_string())?;
    match tokio::time::timeout(CONVERT_TIMEOUT, child.wait_with_output()).await {
        Ok(output) => output.map_err(|e| e.to_string()),
        Err(_) => Err("转换超时".to_string()),
    }
}

#[cfg(target_os = "windows")]
async fn convert_with_office(docx: &Path, pdf: &Path, work_dir: &Path) -> Result<String, String> {
    let script = work_dir.join("to_pdf.ps1");
    std::fs::write(&script, OFFICE_SCRIPT).map_err(|e| e.to_string())?;
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .env("MM_PDF_SRC", docx)
        .env("MM_PDF_DST", pdf);
    let output = run(command).await?;
    if output.status.success() && pdf.exists() {
        let prog_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok(if prog_id.starts_with("Word") { "Microsoft Word".to_string() } else { "WPS".to_string() });
    }
    Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
}

fn libreoffice_candidates() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = ["soffice", "libreoffice"].iter().filter_map(|name| which::which(name).ok()).collect();
    #[cfg(target_os = "windows")]
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(dir) = std::env::var(var) {
            found.push(PathBuf::from(dir).join("LibreOffice").join("program").join("soffice.exe"));
        }
    }
    #[cfg(target_os = "macos")]
    found.push(PathBuf::from("/Applications/LibreOffice.app/Contents/MacOS/soffice"));
    found.into_iter().filter(|path| path.exists()).collect()
}

async fn convert_with_libreoffice(docx: &Path, pdf: &Path, work_dir: &Path) -> Result<String, String> {
    let Some(soffice) = libreoffice_candidates().into_iter().next() else {
        return Err("LibreOffice not found".to_string());
    };
    // A private profile, so a LibreOffice window the user has open does not swallow the job
    let profile = work_dir.join("lo-profile");
    let profile_url = format!("file:///{}", profile.to_string_lossy().replace('\\', "/").trim_start_matches('/'));
    let mut command = Command::new(&soffice);
    command
        .arg(format!("-env:UserInstallation={}", profile_url))
        .args(["--headless", "--convert-to", "pdf", "--outdir"])
        .arg(work_dir)
        .arg(docx);
    let output = run(command).await?;
    let produced = docx.with_extension("pdf");
    if output.status.success() && produced.exists() {
        std::fs::rename(&produced, pdf).or_else(|_| std::fs::copy(&produced, pdf).map(|_| ()))
            .map_err(|e| e.to_string())?;
        return Ok("LibreOffice".to_string());
    }
    Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
}

/// Converts the .docx bytes to a PDF at `pdf`. Returns the name of the office suite that did it.
pub async fn docx_to_pdf(docx_bytes: &[u8], pdf: &Path) -> Result<String, String> {
    let work_dir = tempfile::Builder::new()
        .prefix("minutes-pdf-")
        .tempdir()
        .map_err(|e| format!("创建临时目录失败：{}", e))?;
    let docx = work_dir.path().join("minutes.docx");
    std::fs::write(&docx, docx_bytes).map_err(|e| format!("写入临时文件失败：{}", e))?;
    let _ = std::fs::remove_file(pdf);

    // What went wrong in an office suite that was found, to show instead of "none found"
    #[allow(unused_mut)]
    let mut office_error: Option<String> = None;
    #[cfg(target_os = "windows")]
    match convert_with_office(&docx, pdf, work_dir.path()).await {
        Ok(name) => {
            info!("PDF converted with {}", name);
            return Ok(name);
        }
        Err(e) => {
            warn!("PDF conversion through Word/WPS failed: {}", e);
            if !e.is_empty() {
                office_error = Some(e);
            }
        }
    }

    match convert_with_libreoffice(&docx, pdf, work_dir.path()).await {
        Ok(name) => {
            info!("PDF converted with {}", name);
            Ok(name)
        }
        Err(e) => {
            warn!("PDF conversion through LibreOffice failed: {}", e);
            Err(match office_error {
                Some(error) => format!("转换 PDF 失败：{}。可以先导出 Word 文件，再在 Word 里另存为 PDF。", error),
                None => NO_CONVERTER.to_string(),
            })
        }
    }
}
