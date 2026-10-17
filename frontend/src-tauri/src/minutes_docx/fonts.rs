//! Font precheck and fallback.
//!
//! The official-document fonts (方正小标宋简体, 仿宋_GB2312, 楷体_GB2312) are commercial
//! and cannot ship with the app. Government office machines usually have them; when one is
//! missing the export falls back along a fixed chain and says so, rather than failing.
//! On Windows the installed list is read from the registry, the authoritative source.
//! Elsewhere the check is unavailable and font names are written as-is.

/// Fallback chains, in order of preference
const FONT_FALLBACK: &[(&str, &[&str])] = &[
    ("方正小标宋简体", &["方正小标宋_GBK", "宋体"]),
    ("仿宋_GB2312", &["仿宋", "宋体"]),
    ("楷体_GB2312", &["楷体", "宋体"]),
];

/// Registry names that differ from the names Word uses
const ALIASES: &[(&str, &str)] = &[
    ("simsun", "宋体"),
    ("simhei", "黑体"),
    ("fangsong", "仿宋"),
    ("kaiti", "楷体"),
    ("fangsong_gb2312", "仿宋_GB2312"),
    ("kaiti_gb2312", "楷体_GB2312"),
    ("fzxiaobiaosong-b05s", "方正小标宋简体"),
];

fn normalize(name: &str) -> String {
    let lower = name.replace([' ', '-'], "").to_lowercase();
    ALIASES
        .iter()
        .find(|(alias, _)| alias.replace('-', "") == lower)
        .map(|(_, canonical)| canonical.replace([' ', '-'], "").to_lowercase())
        .unwrap_or(lower)
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
/// Registry value names look like "仿宋_GB2312 (TrueType)" or "宋体 & 新宋体 (TrueType)"
fn registry_names(raw: &str) -> Vec<String> {
    let mut text = raw.trim();
    if let Some(open) = text.rfind('(') {
        let inner = text[open + 1..].trim_end_matches(')').trim().to_lowercase();
        if ["truetype", "opentype", "all res", "type 1", "all res font"].contains(&inner.as_str()) {
            text = text[..open].trim();
        }
    }
    text.split('&').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

pub struct FontResolver {
    /// Normalized names of installed fonts; None when the check is unavailable
    installed: Option<Vec<String>>,
}

impl FontResolver {
    /// No precheck available: font names are used as written
    pub fn unknown() -> Self {
        Self { installed: None }
    }

    pub fn from_installed(names: impl IntoIterator<Item = String>) -> Self {
        let installed: Vec<String> = names.into_iter().map(|n| normalize(&n)).collect();
        Self { installed: (!installed.is_empty()).then_some(installed) }
    }

    /// Read the installed fonts of this machine
    pub fn detect() -> Self {
        Self::from_installed(installed_font_names())
    }

    pub fn is_available(&self) -> bool {
        self.installed.is_some()
    }

    fn is_installed(&self, font: &str) -> bool {
        let target = normalize(font);
        self.installed.as_ref().is_some_and(|list| list.iter().any(|f| *f == target))
    }

    /// The font to write for `font`: itself, or the first installed fallback,
    /// or the end of the chain when nothing in it is installed
    pub fn resolve(&self, font: &str) -> String {
        if self.installed.is_none() || self.is_installed(font) {
            return font.to_string();
        }
        let chain = FONT_FALLBACK.iter().find(|(name, _)| *name == font).map(|(_, chain)| *chain).unwrap_or(&[]);
        chain
            .iter()
            .find(|candidate| self.is_installed(candidate))
            .or(chain.last())
            .map(|s| s.to_string())
            .unwrap_or_else(|| font.to_string())
    }

    /// One warning per required font that is not installed
    pub fn warnings(&self, fonts: &[&str]) -> Vec<String> {
        if self.installed.is_none() {
            return vec![];
        }
        fonts
            .iter()
            .filter(|font| !self.is_installed(font))
            .map(|font| {
                let resolved = self.resolve(font);
                if resolved != *font && self.is_installed(&resolved) {
                    format!("本机没有安装“{}”，已改用“{}”，版式与公文规范略有差异", font, resolved)
                } else {
                    format!("本机没有安装“{}”，也没有可替代的字体，Word 将使用默认字体", font)
                }
            })
            .collect()
    }
}

#[cfg(target_os = "windows")]
fn installed_font_names() -> Vec<String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts";
    let mut names = Vec::new();
    // Fonts installed for the current user only are listed under HKCU
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        if let Ok(key) = RegKey::predef(hive).open_subkey(KEY) {
            for (value_name, _) in key.enum_values().flatten() {
                names.extend(registry_names(&value_name));
            }
        }
    }
    names
}

#[cfg(not(target_os = "windows"))]
fn installed_font_names() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_value_names_are_split_and_cleaned() {
        assert_eq!(registry_names("仿宋_GB2312 (TrueType)"), vec!["仿宋_GB2312"]);
        assert_eq!(registry_names("宋体 & 新宋体 (TrueType)"), vec!["宋体", "新宋体"]);
        assert_eq!(registry_names("SimHei (TrueType)"), vec!["SimHei"]);
    }

    #[test]
    fn english_registry_names_match_chinese_font_names() {
        let fonts = FontResolver::from_installed(["SimHei".to_string(), "FangSong".to_string()]);
        assert!(fonts.is_installed("黑体"));
        assert_eq!(fonts.resolve("仿宋_GB2312"), "仿宋");
    }

    #[test]
    fn unknown_keeps_names() {
        let fonts = FontResolver::unknown();
        assert_eq!(fonts.resolve("仿宋_GB2312"), "仿宋_GB2312");
        assert!(fonts.warnings(&["仿宋_GB2312"]).is_empty());
    }
}
