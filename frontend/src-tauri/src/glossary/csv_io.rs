//! Glossary CSV files. Reads the earlier workbench's layout (`category,canonical,aliases,note`
//! plus optional `entry_id,enabled`) and the Chinese layout this app writes
//! (`类别,规范写法,易错写法,备注,启用`). Files saved by Excel on Chinese Windows are often GBK,
//! so that is accepted too.

use super::matcher::{category_label, parse_aliases, parse_category, validate_entry, GlossaryEntry};

const HEADER_ALIASES: &[(&str, &[&str])] = &[
    ("category", &["category", "类别", "分类"]),
    ("canonical", &["canonical", "规范写法", "正确写法"]),
    ("aliases", &["aliases", "易错写法", "别名", "错误写法"]),
    ("note", &["note", "备注", "说明"]),
    ("entry_id", &["entry_id", "编号"]),
    ("enabled", &["enabled", "启用"]),
];

/// A row read from a file; `id` is empty unless the file had one
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvRow {
    pub line: usize,
    pub entry: GlossaryEntry,
}

fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::GBK.decode(bytes).0.into_owned(),
    }
}

fn is_disabled(raw: &str) -> bool {
    matches!(raw.trim().to_lowercase().as_str(), "0" | "false" | "no" | "否" | "停用" | "n")
}

/// Rows that parsed, and a problem list naming each rejected row by its line in the file
pub fn parse_csv(bytes: &[u8]) -> (Vec<CsvRow>, Vec<String>) {
    let text = decode(bytes);
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());

    let headers = match reader.headers() {
        Ok(h) => h.clone(),
        Err(e) => return (Vec::new(), vec![format!("读不出表头：{}", e)]),
    };
    let column = |field: &str| -> Option<usize> {
        let names = HEADER_ALIASES.iter().find(|(f, _)| *f == field).map(|(_, n)| *n)?;
        headers.iter().position(|h| names.contains(&h.trim().to_lowercase().as_str()))
    };
    let (Some(cat_col), Some(canon_col)) = (column("category"), column("canonical")) else {
        return (
            Vec::new(),
            vec!["表头缺少「类别」或「规范写法」列（也可以用 category、canonical）".to_string()],
        );
    };
    let alias_col = column("aliases");
    let note_col = column("note");
    let id_col = column("entry_id");
    let enabled_col = column("enabled");

    let mut rows = Vec::new();
    let mut problems = Vec::new();
    let mut seen: Vec<(String, String, usize)> = Vec::new();
    for record in reader.records() {
        let record = match record {
            Ok(r) => r,
            Err(e) => {
                problems.push(format!("有一行格式不对：{}", e));
                continue;
            }
        };
        let line = record.position().map(|p| p.line() as usize).unwrap_or(0);
        let get = |col: Option<usize>| col.and_then(|c| record.get(c)).unwrap_or("").trim().to_string();
        let category_raw = get(Some(cat_col));
        let canonical = get(Some(canon_col));
        if category_raw.is_empty() && canonical.is_empty() {
            continue;
        }
        if category_raw.starts_with('#') || canonical.starts_with('#') {
            continue;
        }
        let Some(category) = parse_category(&category_raw) else {
            problems.push(format!(
                "第 {} 行：类别「{}」无效，只能是机构名、人名职务、业务术语或项目代号",
                line, category_raw
            ));
            continue;
        };
        if canonical.is_empty() {
            problems.push(format!("第 {} 行：缺少规范写法", line));
            continue;
        }
        if let Some((_, _, first)) = seen.iter().find(|(c, n, _)| c == category && *n == canonical) {
            problems.push(format!("第 {} 行：与第 {} 行重复（同一类别下规范写法不能重复）", line, first));
            continue;
        }
        let aliases = match parse_aliases(&get(alias_col)) {
            Ok(a) => a,
            Err(e) => {
                problems.push(format!("第 {} 行：{}", line, e));
                continue;
            }
        };
        let entry = GlossaryEntry {
            id: get(id_col),
            category: category.to_string(),
            canonical: canonical.clone(),
            aliases,
            note: get(note_col),
            enabled: !is_disabled(&get(enabled_col)),
        };
        let entry_problems = validate_entry(&entry);
        if !entry_problems.is_empty() {
            problems.extend(entry_problems.into_iter().map(|p| format!("第 {} 行：{}", line, p)));
            continue;
        }
        seen.push((category.to_string(), canonical, line));
        rows.push(CsvRow { line, entry });
    }
    (rows, problems)
}

/// UTF-8 with a byte order mark, so Excel opens it with the right encoding
pub fn write_csv(entries: &[GlossaryEntry]) -> Vec<u8> {
    let mut out = vec![0xEF, 0xBB, 0xBF];
    {
        let mut writer = csv::Writer::from_writer(&mut out);
        let _ = writer.write_record(["类别", "规范写法", "易错写法", "备注", "启用"]);
        for e in entries {
            let _ = writer.write_record([
                category_label(&e.category).unwrap_or(&e.category),
                &e.canonical,
                &e.aliases.join("|"),
                &e.note,
                if e.enabled { "是" } else { "否" },
            ]);
        }
        let _ = writer.flush();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_earlier_workbench_layout() {
        let csv = "category,canonical,aliases,note\n\
                   org,中共中央办公厅,中办|中央办公厅,\n\
                   # 注释,,,\n\
                   person,张建国,张建军,办公室主任\n";
        let (rows, problems) = parse_csv(csv.as_bytes());
        assert!(problems.is_empty(), "{:?}", problems);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].entry.aliases, vec!["中办", "中央办公厅"]);
        assert_eq!(rows[1].entry.note, "办公室主任");
        assert!(rows[1].entry.enabled);
    }

    #[test]
    fn reads_chinese_headers_labels_and_gbk() {
        let csv = "类别,规范写法,易错写法,备注,启用\n人名职务,张建国,张建军|张见国,,否\n";
        let (gbk, _, _) = encoding_rs::GBK.encode(csv);
        let (rows, problems) = parse_csv(&gbk);
        assert!(problems.is_empty(), "{:?}", problems);
        assert_eq!(rows[0].entry.category, "person");
        assert_eq!(rows[0].entry.aliases.len(), 2);
        assert!(!rows[0].entry.enabled);
    }

    #[test]
    fn reports_bad_rows_by_line() {
        let csv = "类别,规范写法,易错写法\n人物,张建国,\n人名职务,,张三\n人名职务,张建国,张\n人名职务,李四,李斯\n人名职务,李四,李司\n";
        let (rows, problems) = parse_csv(csv.as_bytes());
        assert_eq!(rows.len(), 1);
        assert_eq!(problems.len(), 4, "{:?}", problems);
        assert!(problems[0].starts_with("第 2 行"));
        assert!(problems[3].contains("与第 5 行重复"));
    }

    #[test]
    fn round_trips() {
        let entries = vec![GlossaryEntry {
            id: "x".into(),
            category: "org".into(),
            canonical: "办公厅".into(),
            aliases: vec!["办工厅".into(), "办公听".into()],
            note: "含,逗号".into(),
            enabled: false,
        }];
        let (rows, problems) = parse_csv(&write_csv(&entries));
        assert!(problems.is_empty());
        let mut back = rows[0].entry.clone();
        back.id = "x".into();
        assert_eq!(back, entries[0]);
    }
}
