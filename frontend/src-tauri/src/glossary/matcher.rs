//! Glossary rules, ported from the earlier workbench (`mmwf/domain/glossary/matcher.py`).
//! No database or Tauri dependency so it can be tested on its own.
//!
//! A glossary entry is a correct spelling (`canonical`) and the ways speech recognition gets it
//! wrong (`aliases`). After transcription every alias is replaced by its canonical form:
//! - Only exact replacements, no fuzzy or pinyin matching: changing one name into another is
//!   worse than leaving a typo for the reviewer.
//! - Longer aliases are replaced first, so a short alias cannot cut up a longer one.
//! - Text that is already correct is never touched: canonical forms are masked before replacing,
//!   so the alias 张三 does not turn the canonical 张三丰 into 李四丰. This also makes a second
//!   run change nothing.
//! - Document numbers such as 中办发〔2026〕1号 are masked too, so an alias can never alter one.
//! - Replaced text is masked as soon as it is written, so a shorter alias cannot match inside a
//!   canonical form just put in.
//! - Every replacement is reported, never applied silently.

use std::collections::{BTreeMap, HashMap};

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// (category, label) in the order terms go into the transcription prompt: names and
/// organisations are the words recognition most needs help with.
pub const CATEGORIES: &[(&str, &str)] = &[
    ("person", "人名职务"),
    ("org", "机构名"),
    ("codename", "项目代号"),
    ("term", "业务术语"),
];

pub fn category_label(category: &str) -> Option<&'static str> {
    CATEGORIES.iter().find(|(c, _)| *c == category).map(|(_, l)| *l)
}

/// Accepts the code (`org`) or the label (`机构名`)
pub fn parse_category(raw: &str) -> Option<&'static str> {
    let raw = raw.trim();
    CATEGORIES
        .iter()
        .find(|(c, l)| *c == raw || *l == raw)
        .map(|(c, _)| *c)
}

fn category_rank(category: &str) -> usize {
    CATEGORIES.iter().position(|(c, _)| *c == category).unwrap_or(CATEGORIES.len())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryEntry {
    pub id: String,
    pub category: String,
    pub canonical: String,
    pub aliases: Vec<String>,
    #[serde(default)]
    pub note: String,
    pub enabled: bool,
}

/// Aliases written as `a|b|c` (CSV) or as a JSON array (database). Blank items are dropped
/// (a blank alias would match everywhere); duplicates are dropped keeping the first.
pub fn parse_aliases(raw: &str) -> Result<Vec<String>, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(Vec::new());
    }
    let items: Vec<String> = if s.starts_with('[') || s.starts_with('{') {
        match serde_json::from_str::<serde_json::Value>(s) {
            Ok(serde_json::Value::Array(values)) => values
                .into_iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                })
                .collect(),
            Ok(_) => return Err(format!("易错写法的 JSON 必须是数组：{}", s)),
            Err(_) => return Err(format!("易错写法不是合法的 JSON 数组：{}", s)),
        }
    } else {
        s.split('|').map(str::to_string).collect()
    };
    Ok(dedup_aliases(items))
}

pub fn dedup_aliases(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in items {
        let a = a.trim().to_string();
        if !a.is_empty() && !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

/// Problems with one entry; empty means it is fine
pub fn validate_entry(entry: &GlossaryEntry) -> Vec<String> {
    let mut problems = Vec::new();
    if category_label(&entry.category).is_none() {
        problems.push(format!("类别「{}」无效，只能是机构名、人名职务、业务术语或项目代号", entry.category));
    }
    if entry.canonical.trim().is_empty() {
        problems.push("规范写法不能为空".to_string());
    } else if entry.canonical.trim() != entry.canonical {
        problems.push(format!("规范写法首尾有空白：「{}」", entry.canonical));
    }
    if has_reserved_chars(&entry.canonical) {
        problems.push("规范写法含有不可见的控制字符".to_string());
    }
    for a in &entry.aliases {
        if *a == entry.canonical {
            problems.push(format!("易错写法「{}」与规范写法相同", a));
        } else if a.trim() != a {
            problems.push(format!("易错写法首尾有空白：「{}」", a));
        } else if a.chars().count() < 2 {
            // One character would match inside ordinary words all over the transcript
            problems.push(format!("易错写法「{}」只有一个字，会误改正文，至少要两个字", a));
        } else if has_reserved_chars(a) {
            problems.push("易错写法含有不可见的控制字符".to_string());
        }
    }
    problems
}

/// alias -> (canonical, entry id); canonical forms map to themselves
pub type AliasIndex = BTreeMap<String, (String, String)>;

/// When two entries claim the same alias the choice must not depend on the order entries were
/// read in, or the same transcript and glossary could give different results. The longer
/// canonical form wins (it is usually the fuller name), then the smaller one, then the entry id.
/// [`detect_conflicts`] reports these so the user can fix the glossary.
pub fn build_alias_index(entries: &[GlossaryEntry]) -> AliasIndex {
    let mut groups: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for e in entries.iter().filter(|e| e.enabled) {
        for a in &e.aliases {
            if !a.trim().is_empty() {
                groups.entry(a.clone()).or_default().push((e.canonical.clone(), e.id.clone()));
            }
        }
    }
    let mut index: AliasIndex = groups
        .into_iter()
        .map(|(alias, cands)| {
            let best = cands
                .into_iter()
                .min_by(|a, b| {
                    b.0.chars().count()
                        .cmp(&a.0.chars().count())
                        .then_with(|| a.0.cmp(&b.0))
                        .then_with(|| a.1.cmp(&b.1))
                })
                .expect("non-empty group");
            (alias, best)
        })
        .collect();
    for e in entries.iter().filter(|e| e.enabled) {
        let c = e.canonical.trim();
        if !c.is_empty() {
            index.entry(c.to_string()).or_insert_with(|| (c.to_string(), e.id.clone()));
        }
    }
    index
}

/// alias -> every canonical form it points to, for aliases claimed by more than one
pub fn detect_conflicts(entries: &[GlossaryEntry]) -> BTreeMap<String, Vec<String>> {
    let mut targets: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in entries.iter().filter(|e| e.enabled) {
        for a in &e.aliases {
            let bucket = targets.entry(a.clone()).or_default();
            if !bucket.contains(&e.canonical) {
                bucket.push(e.canonical.clone());
            }
        }
    }
    targets.retain(|_, v| v.len() > 1);
    targets
}

/// Longest first; equal lengths in string order, so results are reproducible
pub fn sort_longest_first<'a>(items: impl IntoIterator<Item = &'a String>) -> Vec<&'a String> {
    let mut v: Vec<&String> = items.into_iter().collect();
    v.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()).then_with(|| a.cmp(b)));
    v.dedup();
    v
}

/// Words that end a sentence fragment rather than start a document number's issuer code;
/// keeps "据中办发〔2026〕1号" from swallowing 据 into the protected span.
const FILE_NUMBER_STOP_CHARS: &str = "了的和与至于及在是为第据经由见称等此该本份次项上下前后就都也又还再";

static FILE_NUMBER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"[^{}\s〔〕\[\]（）()]{{2,7}}[〔\[（(]\s*\d{{4}}\s*[〕\]）)]\s*\d{{1,4}}\s*号",
        FILE_NUMBER_STOP_CHARS
    ))
    .expect("file number pattern")
});

// Placeholders are made of private-use characters, which no alias can contain (see
// [`has_reserved_chars`]); an ASCII placeholder such as C1 could be hit by an alias like C1.
const MARK: char = '\u{E000}';
const NS_FILE: char = '\u{E001}';
const NS_CANON: char = '\u{E002}';
const NS_REPLACED: char = '\u{E003}';

fn placeholder(ns: char, n: usize) -> String {
    let digits: String = n
        .to_string()
        .chars()
        .map(|d| char::from_u32(0xE010 + d.to_digit(10).unwrap_or(0)).unwrap_or(MARK))
        .collect();
    format!("{MARK}{ns}{digits}{MARK}")
}

fn has_reserved_chars(s: &str) -> bool {
    s.chars().any(|c| ('\u{E000}'..='\u{E0FF}').contains(&c) || c == '\0')
}

fn unmask(text: &str, ns: char, originals: &[String]) -> String {
    let mut out = text.to_string();
    // Highest number first so placeholder 1 is not confused with the start of placeholder 11
    for i in (0..originals.len()).rev() {
        out = out.replace(&placeholder(ns, i), &originals[i]);
    }
    out
}

/// Every start position of `needle` in `hay`, overlapping matches included
fn find_all(hay: &str, needle: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(i) = hay[start..].find(needle) {
        let at = start + i;
        out.push(at);
        start = at + hay[at..].chars().next().map(char::len_utf8).unwrap_or(1);
    }
    out
}

fn mask_file_numbers(text: &str) -> (String, Vec<String>) {
    let mut originals = Vec::new();
    let masked = FILE_NUMBER
        .replace_all(text, |caps: &regex::Captures| {
            originals.push(caps[0].to_string());
            placeholder(NS_FILE, originals.len() - 1)
        })
        .into_owned();
    (masked, originals)
}

/// Masks each occurrence of a canonical form unless it lies inside an occurrence of an alias:
/// with canonical 某单位 and alias 某单位简称, masking 某单位 would stop the alias from matching.
fn mask_canonicals(text: &str, canonicals: &[&String], aliases: &[&String]) -> (String, Vec<String>) {
    let mut out = text.to_string();
    let mut originals = Vec::new();
    for v in sort_longest_first(canonicals.iter().copied()) {
        if v.is_empty() || !out.contains(v.as_str()) {
            continue;
        }
        let mut pieces = String::with_capacity(out.len());
        let mut cursor = 0;
        // Measured on the text as masked so far, so a longer canonical already masked
        // hides the aliases inside it
        let alias_spans: Vec<(usize, usize)> = aliases
            .iter()
            .flat_map(|a| find_all(&out, a).into_iter().map(move |i| (i, i + a.len())))
            .collect();
        while let Some(rel) = out[cursor..].find(v.as_str()) {
            let i = cursor + rel;
            pieces.push_str(&out[cursor..i]);
            if alias_spans.iter().any(|&(s, e)| s <= i && i + v.len() <= e) {
                pieces.push_str(v);
            } else {
                pieces.push_str(&placeholder(NS_CANON, originals.len()));
                originals.push(v.clone());
            }
            cursor = i + v.len();
        }
        pieces.push_str(&out[cursor..]);
        out = pieces;
    }
    (out, originals)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionHit {
    pub alias: String,
    pub canonical: String,
    pub entry_id: String,
    pub count: usize,
}

/// Replaces aliases with their canonical form. Returns the new text and one hit per alias found.
pub fn apply_corrections(text: &str, index: &AliasIndex) -> (String, Vec<CorrectionHit>) {
    let real_aliases: Vec<&String> = index.iter().filter(|(a, (c, _))| *a != c).map(|(a, _)| a).collect();
    if text.is_empty() || real_aliases.is_empty() {
        return (text.to_string(), Vec::new());
    }
    if !real_aliases.iter().any(|a| text.contains(a.as_str())) {
        return (text.to_string(), Vec::new());
    }

    let (working, file_numbers) = mask_file_numbers(text);
    let mut canonicals: Vec<&String> = index.values().map(|(c, _)| c).collect();
    canonicals.sort();
    canonicals.dedup();
    let (mut working, shielded) = mask_canonicals(&working, &canonicals, &real_aliases);

    let mut replaced: Vec<String> = Vec::new();
    let mut slot_of: HashMap<&str, usize> = HashMap::new();
    let mut hits = Vec::new();
    for alias in sort_longest_first(real_aliases.iter().copied()) {
        let n = working.matches(alias.as_str()).count();
        if n == 0 {
            continue;
        }
        let (canonical, entry_id) = &index[alias];
        let slot = *slot_of.entry(canonical.as_str()).or_insert_with(|| {
            replaced.push(canonical.clone());
            replaced.len() - 1
        });
        working = working.replace(alias.as_str(), &placeholder(NS_REPLACED, slot));
        hits.push(CorrectionHit {
            alias: alias.clone(),
            canonical: canonical.clone(),
            entry_id: entry_id.clone(),
            count: n,
        });
    }

    let working = unmask(&working, NS_REPLACED, &replaced);
    let working = unmask(&working, NS_CANON, &shielded);
    let working = unmask(&working, NS_FILE, &file_numbers);
    (working, hits)
}

/// Adds one text's hits into a running total
pub fn merge_hits(total: &mut Vec<CorrectionHit>, more: Vec<CorrectionHit>) {
    for h in more {
        match total.iter_mut().find(|t| t.alias == h.alias && t.canonical == h.canonical) {
            Some(t) => t.count += h.count,
            None => total.push(h),
        }
    }
}

/// Canonical forms for the transcription prompt, names and organisations first, until
/// `budget_chars` characters are used. Whisper reads at most about 224 prompt tokens and
/// ignores the rest without telling anyone, so the list is cut here instead.
/// Returns (terms used, terms available).
pub fn prompt_terms(entries: &[GlossaryEntry], budget_chars: usize) -> (Vec<String>, usize) {
    let mut ordered: Vec<(usize, usize, &str)> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.enabled && !e.canonical.trim().is_empty())
        .map(|(i, e)| (category_rank(&e.category), i, e.canonical.trim()))
        .collect();
    ordered.sort();
    let mut seen: Vec<&str> = Vec::new();
    for (_, _, c) in ordered {
        if !seen.contains(&c) {
            seen.push(c);
        }
    }
    let total = seen.len();
    let mut used = Vec::new();
    let mut chars = 0;
    for c in seen {
        let n = c.chars().count() + 1; // with the 、 separator
        if chars + n > budget_chars {
            continue;
        }
        chars += n;
        used.push(c.to_string());
    }
    (used, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, category: &str, canonical: &str, aliases: &[&str]) -> GlossaryEntry {
        GlossaryEntry {
            id: id.to_string(),
            category: category.to_string(),
            canonical: canonical.to_string(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            note: String::new(),
            enabled: true,
        }
    }

    fn correct(entries: &[GlossaryEntry], text: &str) -> (String, usize) {
        let (out, hits) = apply_corrections(text, &build_alias_index(entries));
        (out, hits.iter().map(|h| h.count).sum())
    }

    #[test]
    fn replaces_aliases_and_reports_hits() {
        let e = vec![entry("1", "person", "张建国", &["张建军", "张见国"])];
        let (out, n) = correct(&e, "张建军说，张见国同意。张建国补充。");
        assert_eq!(out, "张建国说，张建国同意。张建国补充。");
        assert_eq!(n, 2);
    }

    #[test]
    fn canonical_text_is_not_cut_by_a_shorter_alias() {
        let e = vec![
            entry("1", "person", "李四", &["张三"]),
            entry("2", "person", "张三丰", &[]),
        ];
        let (out, n) = correct(&e, "张三丰和张三");
        assert_eq!(out, "张三丰和李四");
        assert_eq!(n, 1);
    }

    #[test]
    fn alias_containing_its_canonical_still_matches() {
        let e = vec![entry("1", "org", "某单位", &["某单位简称"])];
        let (out, n) = correct(&e, "某单位简称和某单位");
        assert_eq!(out, "某单位和某单位");
        assert_eq!(n, 1);
    }

    #[test]
    fn longer_alias_wins() {
        let e = vec![
            entry("1", "org", "中共中央办公厅", &["中办"]),
            entry("2", "org", "国务院办公厅", &["中办公厅"]),
        ];
        let (out, _) = correct(&e, "中办公厅通知");
        assert_eq!(out, "国务院办公厅通知");
    }

    #[test]
    fn document_numbers_are_protected() {
        let e = vec![entry("1", "org", "中共中央办公厅", &["中办"])];
        let (out, n) = correct(&e, "根据中办发〔2026〕1号文件，中办要求");
        assert_eq!(out, "根据中办发〔2026〕1号文件，中共中央办公厅要求");
        assert_eq!(n, 1);
        let (out, _) = correct(&e, "据中办发[2026]12号");
        assert_eq!(out, "据中办发[2026]12号");
    }

    #[test]
    fn replaced_text_is_not_matched_again() {
        let e = vec![
            entry("1", "person", "王建国", &["王见国"]),
            entry("2", "term", "兴邦", &["建国"]),
        ];
        let (out, _) = correct(&e, "王见国");
        assert_eq!(out, "王建国");
    }

    #[test]
    fn ascii_aliases_cannot_hit_placeholders() {
        let e = vec![
            entry("1", "codename", "甲工程", &["C1", "E0"]),
            entry("2", "org", "办公厅", &["办工厅"]),
        ];
        let (out, n) = correct(&e, "办公厅和办工厅讨论C1");
        assert_eq!(out, "办公厅和办公厅讨论甲工程");
        assert_eq!(n, 2);
    }

    #[test]
    fn second_run_changes_nothing() {
        let e = vec![
            entry("1", "org", "中共中央办公厅", &["中办"]),
            entry("2", "person", "张建国", &["张建军"]),
        ];
        let (once, _) = correct(&e, "中办张建军");
        let (twice, n) = correct(&e, &once);
        assert_eq!(once, twice);
        assert_eq!(n, 0);
    }

    #[test]
    fn disabled_entries_do_nothing() {
        let mut e = entry("1", "person", "张建国", &["张建军"]);
        e.enabled = false;
        assert_eq!(correct(&[e], "张建军").0, "张建军");
    }

    #[test]
    fn conflicts_resolve_the_same_way_in_any_order() {
        let a = entry("a", "org", "中共中央办公厅", &["中办"]);
        let b = entry("b", "org", "中国办公室", &["中办"]);
        let one = correct(&[a.clone(), b.clone()], "中办").0;
        let two = correct(&[b.clone(), a.clone()], "中办").0;
        assert_eq!(one, "中共中央办公厅");
        assert_eq!(one, two);
        assert_eq!(detect_conflicts(&[a, b]).len(), 1);
    }

    #[test]
    fn parses_aliases() {
        assert_eq!(parse_aliases(" 甲乙| 丙丁 ||甲乙").unwrap(), vec!["甲乙", "丙丁"]);
        assert_eq!(parse_aliases(r#"["甲乙","丙丁"]"#).unwrap(), vec!["甲乙", "丙丁"]);
        assert!(parse_aliases(r#"{"a":1}"#).is_err());
        assert!(parse_aliases("[oops").is_err());
        assert!(parse_aliases("").unwrap().is_empty());
    }

    #[test]
    fn validation_rejects_single_character_aliases() {
        let e = entry("1", "person", "张建国", &["张"]);
        assert_eq!(validate_entry(&e).len(), 1);
        assert!(validate_entry(&entry("1", "person", "张建国", &["张建军"])).is_empty());
        assert_eq!(validate_entry(&entry("1", "people", "张建国", &[])).len(), 1);
    }

    #[test]
    fn prompt_terms_put_names_first_and_respect_the_budget() {
        let e = vec![
            entry("1", "term", "党组会", &[]),
            entry("2", "person", "张建国", &[]),
            entry("3", "org", "办公厅", &[]),
        ];
        let (used, total) = prompt_terms(&e, 8);
        assert_eq!(total, 3);
        assert_eq!(used, vec!["张建国", "办公厅"]);
    }
}
