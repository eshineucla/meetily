//! Chinese typesetting cleanup: only how characters sit next to each other, never what was said.
//!
//! Order matters: add CJK–Latin spaces, convert quotes, collapse spaces, drop spaces between CJK.
//! Unlike the earlier project, digits get no space ("2026年10月15日"), as official documents write dates.

use regex::Regex;
use std::sync::LazyLock;

static CJK_LATIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([\x{4e00}-\x{9fff}\x{3400}-\x{4dbf}])([A-Za-z])").unwrap());
static LATIN_CJK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([A-Za-z])([\x{4e00}-\x{9fff}\x{3400}-\x{4dbf}])").unwrap());
static MULTI_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t]{2,}").unwrap());
static CJK_SPACE_CJK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([\x{4e00}-\x{9fff}\x{3400}-\x{4dbf}])[ \t]+([\x{4e00}-\x{9fff}\x{3400}-\x{4dbf}])").unwrap());

pub fn normalize_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let text = add_cjk_latin_spaces(text);
    let text = convert_quotes(&text);
    let text = MULTI_SPACE.replace_all(&text, " ").into_owned();
    drop_spaces_between_cjk(&text)
}

pub fn add_cjk_latin_spaces(text: &str) -> String {
    let text = CJK_LATIN.replace_all(text, "$1 $2");
    LATIN_CJK.replace_all(&text, "$1 $2").into_owned()
}

/// Straight quotes become curly ones, alternating open/close; apostrophes between letters stay
pub fn convert_quotes(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let (mut double_open, mut single_open) = (0u32, 0u32);
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '"' => {
                if double_open > 0 {
                    out.push('”');
                    double_open -= 1;
                } else {
                    out.push('“');
                    double_open += 1;
                }
            }
            '\'' => {
                let prev = i.checked_sub(1).map(|p| chars[p]);
                let next = chars.get(i + 1).copied();
                let is_apostrophe = prev.is_some_and(|p| p.is_ascii_alphabetic()) && next.is_some_and(|n| n.is_ascii_alphabetic());
                if is_apostrophe {
                    out.push(c);
                } else if single_open > 0 {
                    out.push('’');
                    single_open -= 1;
                } else {
                    out.push('‘');
                    single_open += 1;
                }
            }
            '“' => {
                double_open += 1;
                out.push(c);
            }
            '”' => {
                double_open = double_open.saturating_sub(1);
                out.push(c);
            }
            '‘' => {
                single_open += 1;
                out.push(c);
            }
            '’' => {
                single_open = single_open.saturating_sub(1);
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// "召 开 会 议" -> "召开会议"; repeated because regex matches do not overlap. Never crosses lines.
pub fn drop_spaces_between_cjk(text: &str) -> String {
    let mut current = text.to_string();
    loop {
        let next = CJK_SPACE_CJK.replace_all(&current, "$1$2").into_owned();
        if next == current {
            return current;
        }
        current = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_between_chinese_and_latin() {
        assert_eq!(normalize_text("讨论了API接口，用JSON格式"), "讨论了 API 接口，用 JSON 格式");
        assert_eq!(normalize_text("（API）"), "（API）");
        assert_eq!(normalize_text("2026年10月15日在3楼"), "2026年10月15日在3楼");
    }

    #[test]
    fn quotes_alternate_and_apostrophes_stay() {
        assert_eq!(convert_quotes(r#"他说"好""#), "他说“好”");
        assert_eq!(convert_quotes("meeting's"), "meeting's");
    }

    #[test]
    fn spaces_inside_chinese_are_removed_but_lines_kept() {
        assert_eq!(normalize_text("召 开 会 议"), "召开会议");
        assert_eq!(normalize_text("会议纪要\n\n议定事项"), "会议纪要\n\n议定事项");
        assert_eq!(normalize_text("你好    世界"), "你好世界");
    }
}
