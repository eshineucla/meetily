//! Export meeting minutes as an official-document style .docx (GB/T 9704—2012).
//!
//! Ported from the docx module of the earlier meeting-minutes-workflow project
//! (style table, OOXML patches, text normalization, font precheck), written
//! directly as WordprocessingML so no Python runtime has to ship with the app.
//!
//! Layout rules that are checked on acceptance:
//! - A4, margins top 37 mm / bottom 35 mm / left 28 mm / right 26 mm
//! - title 二号 方正小标宋简体, centered, not bold
//! - headings numbered 一、 / （一） / 1. in 黑体 / 楷体_GB2312 / 仿宋_GB2312 bold, 三号
//! - body 三号 仿宋_GB2312, 22 lines × 28 characters per page, first line indented 2 characters
//! - tables 小四, header 黑体 bold, zero indent, full borders, header row repeats
//! - page number "— 1 —" in 四号 宋体 7 mm below the type area, odd pages right, even pages left
//! - Latin text in Times New Roman; every run sets ascii/hAnsi/eastAsia/cs fonts

pub mod commands;
pub mod fonts;
pub mod normalize;
pub mod template;

use fonts::FontResolver;
use normalize::normalize_text;
use regex::Regex;
use std::io::{Cursor, Write};
use std::sync::LazyLock;

/// Exact line spacing for 22 lines per page: the 225 mm type area is 12756 twips, and
/// 22 × 579 = 12738 still fits, so the 22nd line is not pushed to the next page
const BODY_LINE_SPACING_PT: f64 = 28.95;
/// 28 characters per line: the 156 mm type area is 442.2 pt, but 28 三号 (16 pt) characters
/// take 448 pt, so body characters are condensed by 0.25 pt (5 twips) to 15.75 pt each (441 pt).
/// Plain character spacing renders the same in Word, WPS and LibreOffice, unlike a character grid.
const BODY_CHAR_SPACING_TWIPS: i32 = -5;
/// GB/T 9704 7.5: the page number's top sits 7 mm (397 twips) below the type area. Word measures
/// the footer distance to the bottom of the footer line, so subtract the 18 pt (360 twips) line too.
const PAGE_NUMBER_LINE_TWIPS: u32 = 360;
const FOOTER_DISTANCE_TWIPS: u32 = 1984 - 397 - PAGE_NUMBER_LINE_TWIPS;
const LATIN_FONT: &str = "Times New Roman";

#[derive(Debug, Clone, Copy, PartialEq)]
enum Align {
    Left,
    Center,
    Justify,
}

impl Align {
    fn as_ooxml(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Justify => "both",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct GbStyle {
    font: &'static str,
    size_pt: f64,
    bold: bool,
    align: Align,
    first_line_indent_chars: u32,
    line_spacing_pt: Option<f64>,
    /// Condensed to fit 28 characters per line; titles and tables keep normal spacing
    condensed: bool,
}

const TITLE: GbStyle = GbStyle {
    font: "方正小标宋简体",
    size_pt: 22.0,
    bold: false,
    align: Align::Center,
    first_line_indent_chars: 0,
    line_spacing_pt: None,
    condensed: false,
};
const H1: GbStyle = GbStyle {
    font: "黑体",
    size_pt: 16.0,
    bold: false,
    align: Align::Left,
    first_line_indent_chars: 2,
    line_spacing_pt: Some(BODY_LINE_SPACING_PT),
    condensed: true,
};
const H2: GbStyle = GbStyle { font: "楷体_GB2312", ..H1 };
const H3: GbStyle = GbStyle { font: "仿宋_GB2312", bold: true, ..H1 };
const BODY: GbStyle = GbStyle {
    font: "仿宋_GB2312",
    size_pt: 16.0,
    bold: false,
    align: Align::Justify,
    first_line_indent_chars: 2,
    line_spacing_pt: Some(BODY_LINE_SPACING_PT),
    condensed: true,
};
const TABLE_BODY: GbStyle = GbStyle {
    font: "仿宋_GB2312",
    size_pt: 12.0,
    bold: false,
    align: Align::Left,
    first_line_indent_chars: 0,
    line_spacing_pt: None,
    condensed: false,
};
const TABLE_HEAD: GbStyle = GbStyle { font: "黑体", bold: true, ..TABLE_BODY };
const PAGE_NUMBER_FONT: &str = "宋体";
const PAGE_NUMBER_SIZE_PT: f64 = 14.0;

/// Fonts the export uses, title font first
pub const REQUIRED_FONTS: &[&str] = &["方正小标宋简体", "黑体", "楷体_GB2312", "仿宋_GB2312", "宋体"];

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Title(String),
    /// Level 1..=3, text already carries its number
    Heading(u8, String),
    Paragraph(String),
    /// Marker (○, ☐, ☑, "1.") and text
    ListItem(String, String),
    Quote(String),
    Table(Vec<Vec<String>>),
    Rule,
}

#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// Drop the （原文：“……”） citations and the 原文依据 column used by the pre-finalization check
    pub strip_sources: bool,
    pub normalize_text: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self { strip_sources: true, normalize_text: true }
    }
}

pub struct ExportResult {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Heading numbers
// ---------------------------------------------------------------------------

const CN_DIGITS: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];

/// 1..=99 as Chinese numerals (公文 numbering always uses 二, never 两)
pub fn to_chinese_numeral(n: u32) -> String {
    match n {
        0 => CN_DIGITS[0].to_string(),
        1..=9 => CN_DIGITS[n as usize].to_string(),
        10..=99 => {
            let (tens, ones) = (n / 10, n % 10);
            let mut text = if tens > 1 { CN_DIGITS[tens as usize].to_string() } else { String::new() };
            text.push('十');
            if ones > 0 {
                text.push_str(CN_DIGITS[ones as usize]);
            }
            text
        }
        _ => n.to_string(),
    }
}

fn heading_prefix(level: u8, counter: u32) -> String {
    match level {
        1 => format!("{}、", to_chinese_numeral(counter)),
        2 => format!("（{}）", to_chinese_numeral(counter)),
        _ => format!("{}.", counter),
    }
}

/// A number the heading already carries (一、 （一） (一) 1. 1、), which is replaced so numbering never skips
static HEADING_NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:[一二三四五六七八九十]+、|（[一二三四五六七八九十]+）|\(\s*[一二三四五六七八九十]+\s*\)|\d+\s*[.、])\s*").unwrap()
});

pub fn strip_heading_number(text: &str) -> String {
    HEADING_NUMBER.replace(text, "").trim().to_string()
}

// ---------------------------------------------------------------------------
// Markdown -> blocks
// ---------------------------------------------------------------------------

static HEADING_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(#{1,6})\s+(.*?)\s*$").unwrap());
static TASK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[-*+]\s+\[([ xX])\]\s+(.*)$").unwrap());
static BULLET_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[-*+]\s+(.*)$").unwrap());
static ORDERED_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(\d+)[.、]\s+(.*)$").unwrap());
static QUOTE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*>\s?(.*)$").unwrap());
static RULE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:-{3,}|\*{3,}|_{3,})\s*$").unwrap());
static TABLE_ROW_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\|.*\|\s*$").unwrap());
static SOURCE_CITATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s*[（(]\s*(?:原文|出处|依据)\s*[:：][^）)]*[）)]").unwrap()
});
static SOURCE_HEADER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"原文|出处|依据").unwrap());
static INLINE_MARKUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*|__|`|<br\s*/?>").unwrap());

fn split_table_cells(line: &str) -> Vec<String> {
    let mut line = line.trim();
    line = line.strip_prefix('|').unwrap_or(line);
    line = line.strip_suffix('|').unwrap_or(line);
    let mut cells = Vec::new();
    let mut buf = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            buf.push('|');
            chars.next();
        } else if c == '|' {
            cells.push(buf.trim().to_string());
            buf.clear();
        } else {
            buf.push(c);
        }
    }
    cells.push(buf.trim().to_string());
    cells
}

fn is_separator_row(cells: &[String]) -> bool {
    cells.iter().all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
}

fn clean_inline(text: &str, options: &ExportOptions) -> String {
    let text = INLINE_MARKUP.replace_all(text, "");
    let text = if options.strip_sources { SOURCE_CITATION.replace_all(&text, "") } else { text };
    text.trim().to_string()
}

pub fn parse_markdown(markdown: &str, options: &ExportOptions) -> (Vec<Block>, Vec<String>) {
    let mut blocks = Vec::new();
    let mut warnings = Vec::new();
    let lines: Vec<&str> = markdown.lines().collect();
    let mut seen_title = false;
    let mut counters = [0u32; 3];
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim_end();
        let stripped = line.trim();
        if stripped.is_empty() {
            i += 1;
            continue;
        }

        if TABLE_ROW_RE.is_match(line) {
            let mut raw_rows = Vec::new();
            while i < lines.len() && TABLE_ROW_RE.is_match(lines[i].trim_end()) {
                raw_rows.push(split_table_cells(lines[i]));
                i += 1;
            }
            let mut rows: Vec<Vec<String>> = if raw_rows.len() >= 2 && is_separator_row(&raw_rows[1]) {
                std::iter::once(raw_rows[0].clone()).chain(raw_rows.into_iter().skip(2)).collect()
            } else {
                warnings.push("表格缺少 Markdown 分隔行（| --- |），已按第一行为表头处理".to_string());
                raw_rows
            };
            // The source column is only there for the pre-finalization check
            if options.strip_sources {
                if let Some(col) = rows.first().and_then(|h| h.iter().position(|c| SOURCE_HEADER.is_match(c))) {
                    for row in rows.iter_mut() {
                        if col < row.len() {
                            row.remove(col);
                        }
                    }
                }
            }
            let width = rows.iter().map(Vec::len).max().unwrap_or(0);
            for row in rows.iter_mut() {
                for cell in row.iter_mut() {
                    *cell = clean_inline(cell, options);
                }
                row.resize(width, String::new());
            }
            if width > 0 {
                blocks.push(Block::Table(rows));
            }
            continue;
        }

        if RULE_RE.is_match(line) {
            blocks.push(Block::Rule);
            i += 1;
            continue;
        }

        if let Some(m) = HEADING_RE.captures(line) {
            let md_level = m[1].len();
            let text = clean_inline(&m[2], options);
            i += 1;
            if !seen_title && md_level == 1 {
                seen_title = true;
                blocks.push(Block::Title(text));
                continue;
            }
            let level = if md_level == 1 {
                warnings.push(format!("出现第 2 个一级标题「{}」，已按公文一级标题排版", text));
                1
            } else {
                if md_level - 1 > 3 {
                    warnings.push(format!("标题「{}」层级过深，公文只到三级，已按三级标题排版", text));
                }
                (md_level - 1).min(3) as u8
            };
            let index = level as usize - 1;
            counters[index] += 1;
            for deeper in counters.iter_mut().skip(level as usize) {
                *deeper = 0;
            }
            let prefix = heading_prefix(level, counters[index]);
            blocks.push(Block::Heading(level, format!("{}{}", prefix, strip_heading_number(&text))));
            continue;
        }

        let block = if let Some(m) = TASK_RE.captures(line) {
            let marker = if m[1].eq_ignore_ascii_case("x") { "☑" } else { "☐" };
            Block::ListItem(marker.to_string(), clean_inline(&m[2], options))
        } else if let Some(m) = BULLET_RE.captures(line) {
            Block::ListItem("○".to_string(), clean_inline(&m[1], options))
        } else if let Some(m) = ORDERED_RE.captures(line) {
            Block::ListItem(format!("{}.", &m[1]), clean_inline(&m[2], options))
        } else if let Some(m) = QUOTE_RE.captures(line) {
            Block::Quote(clean_inline(&m[1], options))
        } else {
            Block::Paragraph(clean_inline(stripped, options))
        };
        blocks.push(block);
        i += 1;
    }

    (blocks, warnings)
}

// ---------------------------------------------------------------------------
// Blocks -> WordprocessingML
// ---------------------------------------------------------------------------

fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // Control characters other than tab/newline are not allowed in XML
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Half-points, as w:sz expects
fn half_points(pt: f64) -> u32 {
    (pt * 2.0).round() as u32
}

fn run_properties(east_asia_font: &str, latin_font: &str, size_pt: f64, bold: bool, char_spacing_twips: i32) -> String {
    format!(
        r#"<w:rPr><w:rFonts w:ascii="{latin}" w:hAnsi="{latin}" w:eastAsia="{ea}" w:cs="{latin}"/>{bold}{spacing}<w:sz w:val="{sz}"/><w:szCs w:val="{sz}"/></w:rPr>"#,
        latin = escape_xml(latin_font),
        ea = escape_xml(east_asia_font),
        bold = if bold { r#"<w:b/><w:bCs/>"# } else { r#"<w:b w:val="0"/><w:bCs w:val="0"/>"# },
        spacing = if char_spacing_twips != 0 { format!(r#"<w:spacing w:val="{char_spacing_twips}"/>"#) } else { String::new() },
        sz = half_points(size_pt),
    )
}

/// All eight indent attributes are written so nothing is inherited (tables must not pick up body indent).
/// firstLine carries the same width in twips as firstLineChars, so readers that ignore one still agree.
fn indent(first_line_chars: u32, size_pt: f64) -> String {
    format!(
        r#"<w:ind w:firstLineChars="{}" w:firstLine="{}" w:leftChars="0" w:left="0" w:rightChars="0" w:right="0" w:hangingChars="0" w:hanging="0"/>"#,
        first_line_chars * 100,
        (first_line_chars as f64 * size_pt * 20.0).round() as u32
    )
}

struct Writer<'a> {
    fonts: &'a FontResolver,
    options: &'a ExportOptions,
    body: String,
}

impl Writer<'_> {
    fn text(&self, text: &str) -> String {
        if self.options.normalize_text {
            normalize_text(text).replace('\'', "’")
        } else {
            text.to_string()
        }
    }

    fn paragraph_xml(&self, style: &GbStyle, first_line_chars: u32, text: &str) -> String {
        let spacing = style
            .line_spacing_pt
            .map(|pt| format!(r#"<w:spacing w:before="0" w:after="0" w:line="{}" w:lineRule="exact"/>"#, (pt * 20.0).round() as u32))
            .unwrap_or_else(|| r#"<w:spacing w:before="0" w:after="0"/>"#.to_string());
        let font = self.fonts.resolve(style.font);
        let run = if text.is_empty() {
            String::new()
        } else {
            format!(
                r#"<w:r>{}<w:t xml:space="preserve">{}</w:t></w:r>"#,
                run_properties(&font, LATIN_FONT, style.size_pt, style.bold, if style.condensed { BODY_CHAR_SPACING_TWIPS } else { 0 }),
                escape_xml(&self.text(text))
            )
        };
        format!(
            r#"<w:p><w:pPr>{}{}<w:jc w:val="{}"/></w:pPr>{}</w:p>"#,
            spacing,
            indent(first_line_chars, style.size_pt),
            style.align.as_ooxml(),
            run
        )
    }

    fn paragraph(&mut self, style: &GbStyle, text: &str) {
        let xml = self.paragraph_xml(style, style.first_line_indent_chars, text);
        self.body.push_str(&xml);
    }

    fn table(&mut self, rows: &[Vec<String>]) {
        let cols = rows.first().map(Vec::len).unwrap_or(0);
        if cols == 0 {
            return;
        }
        // Type area is 156 mm wide = 8844 twips, split evenly
        let col_width = 8844 / cols as u32;
        let mut xml = String::from(
            r#"<w:tbl><w:tblPr><w:tblW w:w="5000" w:type="pct"/><w:tblInd w:w="0" w:type="dxa"/><w:tblBorders>"#,
        );
        for edge in ["top", "left", "bottom", "right", "insideH", "insideV"] {
            xml.push_str(&format!(r#"<w:{edge} w:val="single" w:sz="4" w:space="0" w:color="000000"/>"#));
        }
        xml.push_str(r#"</w:tblBorders><w:tblLayout w:type="fixed"/><w:tblCellMar><w:top w:w="28" w:type="dxa"/><w:start w:w="85" w:type="dxa"/><w:bottom w:w="28" w:type="dxa"/><w:end w:w="85" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid>"#);
        for _ in 0..cols {
            xml.push_str(&format!(r#"<w:gridCol w:w="{col_width}"/>"#));
        }
        xml.push_str("</w:tblGrid>");
        for (r, row) in rows.iter().enumerate() {
            let style = if r == 0 { &TABLE_HEAD } else { &TABLE_BODY };
            xml.push_str("<w:tr>");
            if r == 0 && rows.len() > 1 {
                xml.push_str(r#"<w:trPr><w:tblHeader/></w:trPr>"#);
            }
            for cell in row {
                xml.push_str(&format!(r#"<w:tc><w:tcPr><w:tcW w:w="{col_width}" w:type="dxa"/></w:tcPr>"#));
                xml.push_str(&self.paragraph_xml(style, 0, cell));
                xml.push_str("</w:tc>");
            }
            xml.push_str("</w:tr>");
        }
        xml.push_str("</w:tbl>");
        self.body.push_str(&xml);
        // Two tables with no paragraph between them merge into one in Word
        self.paragraph(&BODY, "");
    }
}

fn document_xml(body: &str) -> String {
    // A4: 11906 x 16838 twips; margins 37/35/28/26 mm
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>{body}<w:sectPr><w:footerReference w:type="default" r:id="rIdFooter1"/><w:footerReference w:type="even" r:id="rIdFooter2"/><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="2098" w:right="1474" w:bottom="1984" w:left="1588" w:header="851" w:footer="{footer}" w:gutter="0"/><w:docGrid w:type="lines" w:linePitch="{pitch}"/></w:sectPr></w:body></w:document>"#,
        footer = FOOTER_DISTANCE_TWIPS,
        pitch = (BODY_LINE_SPACING_PT * 20.0).round() as u32,
    )
}

/// Page number "— 1 —" in 四号 宋体: odd pages on the right, even pages on the left,
/// each one character (14 pt = 280 twips) in from the edge of the type area (GB/T 9704 7.5)
fn footer_xml(font: &str, odd: bool) -> String {
    let rpr = run_properties(font, font, PAGE_NUMBER_SIZE_PT, false, 0);
    let (jc, left, right) = if odd { ("right", 0, 280) } else { ("left", 280, 0) };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="{line}" w:lineRule="exact"/><w:ind w:firstLineChars="0" w:firstLine="0" w:left="{left}" w:right="{right}"/><w:jc w:val="{jc}"/></w:pPr><w:r>{rpr}<w:t xml:space="preserve">— </w:t></w:r><w:r>{rpr}<w:fldChar w:fldCharType="begin"/></w:r><w:r>{rpr}<w:instrText xml:space="preserve"> PAGE </w:instrText></w:r><w:r>{rpr}<w:fldChar w:fldCharType="separate"/></w:r><w:r>{rpr}<w:t>1</w:t></w:r><w:r>{rpr}<w:fldChar w:fldCharType="end"/></w:r><w:r>{rpr}<w:t xml:space="preserve"> —</w:t></w:r></w:p></w:ftr>"#,
        line = PAGE_NUMBER_LINE_TWIPS,
    )
}

fn styles_xml(body_font: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault>{}</w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:before="0" w:after="0"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style></w:styles>"#,
        run_properties(body_font, LATIN_FONT, BODY.size_pt, false, 0)
    )
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/><Override PartName="/word/footer2.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/></Types>"#;

/// Odd and even pages get different footers (page number right / left)
const SETTINGS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:evenAndOddHeaders/></w:settings>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

const DOCUMENT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rIdSettings" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/><Relationship Id="rIdFooter1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/><Relationship Id="rIdFooter2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer2.xml"/></Relationships>"#;

/// Render blocks into the WordprocessingML body
fn render_body(blocks: &[Block], fonts: &FontResolver, options: &ExportOptions) -> String {
    let mut writer = Writer { fonts, options, body: String::new() };
    for block in blocks {
        match block {
            Block::Title(text) => {
                writer.paragraph(&TITLE, text);
                // Blank line between title and body
                writer.paragraph(&BODY, "");
            }
            Block::Heading(level, text) => {
                let style = match level {
                    1 => &H1,
                    2 => &H2,
                    _ => &H3,
                };
                writer.paragraph(style, text);
            }
            Block::Paragraph(text) | Block::Quote(text) => writer.paragraph(&BODY, text),
            // List items are not indented, or the marker would sit oddly
            Block::ListItem(marker, text) => {
                let xml = writer.paragraph_xml(&BODY, 0, &format!("{} {}", marker, text));
                writer.body.push_str(&xml);
            }
            Block::Table(rows) => writer.table(rows),
            Block::Rule => writer.paragraph(&BODY, ""),
        }
    }
    writer.body
}

/// Render markdown minutes into a .docx file
pub fn render_markdown_to_docx(markdown: &str, fonts: &FontResolver, options: &ExportOptions) -> Result<ExportResult, String> {
    let (blocks, mut warnings) = parse_markdown(markdown, options);
    warnings.extend(fonts.warnings(REQUIRED_FONTS));

    let body = render_body(&blocks, fonts, options);
    let page_number_font = fonts.resolve(PAGE_NUMBER_FONT);
    let parts: [(&str, String); 8] = [
        ("[Content_Types].xml", CONTENT_TYPES.to_string()),
        ("_rels/.rels", ROOT_RELS.to_string()),
        ("word/_rels/document.xml.rels", DOCUMENT_RELS.to_string()),
        ("word/document.xml", document_xml(&body)),
        ("word/styles.xml", styles_xml(&fonts.resolve(BODY.font))),
        ("word/settings.xml", SETTINGS.to_string()),
        ("word/footer1.xml", footer_xml(&page_number_font, true)),
        ("word/footer2.xml", footer_xml(&page_number_font, false)),
    ];

    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let file_options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, content) in parts {
        zip.start_file(name, file_options).map_err(|e| e.to_string())?;
        zip.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
    }
    let bytes = zip.finish().map_err(|e| e.to_string())?.into_inner();
    Ok(ExportResult { bytes, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> ExportOptions {
        ExportOptions::default()
    }

    #[test]
    fn chinese_numerals() {
        assert_eq!(to_chinese_numeral(1), "一");
        assert_eq!(to_chinese_numeral(10), "十");
        assert_eq!(to_chinese_numeral(12), "十二");
        assert_eq!(to_chinese_numeral(20), "二十");
        assert_eq!(to_chinese_numeral(99), "九十九");
    }

    #[test]
    fn headings_are_renumbered_and_nested_counters_reset() {
        let md = "# 关于预算的会议纪要\n## 一、会议概况\n### 时间\n### （二）地点\n## 议定事项\n### 第一项";
        let (blocks, warnings) = parse_markdown(md, &opts());
        assert!(warnings.is_empty());
        assert_eq!(
            blocks,
            vec![
                Block::Title("关于预算的会议纪要".into()),
                Block::Heading(1, "一、会议概况".into()),
                Block::Heading(2, "（一）时间".into()),
                Block::Heading(2, "（二）地点".into()),
                Block::Heading(1, "二、议定事项".into()),
                Block::Heading(2, "（一）第一项".into()),
            ]
        );
    }

    #[test]
    fn source_citations_and_column_are_dropped() {
        let md = "1. 会议同意预算方案。（原文：“就按这个方案”）\n\n| 序号 | 工作事项 | 原文依据 |\n| --- | --- | --- |\n| 1 | **编制预算** | “月底前报上来” |";
        let (blocks, _) = parse_markdown(md, &opts());
        assert_eq!(blocks[0], Block::ListItem("1.".into(), "会议同意预算方案。".into()));
        assert_eq!(
            blocks[1],
            Block::Table(vec![vec!["序号".into(), "工作事项".into()], vec!["1".into(), "编制预算".into()]])
        );

        let keep = ExportOptions { strip_sources: false, ..opts() };
        let (blocks, _) = parse_markdown(md, &keep);
        assert!(matches!(&blocks[0], Block::ListItem(_, text) if text.contains("原文")));
    }

    #[test]
    fn table_without_separator_is_accepted_with_warning() {
        let (blocks, warnings) = parse_markdown("| a | b |\n| c |", &opts());
        assert_eq!(blocks, vec![Block::Table(vec![vec!["a".into(), "b".into()], vec!["c".into(), "".into()]])]);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn docx_has_gb_layout() {
        let fonts = FontResolver::unknown();
        let md = "# 测试纪要\n## 会议概况\n会议讨论了API接口。\n\n| 事项 | 责任人 |\n| --- | --- |\n| 起草 | 办公室 |";
        let result = render_markdown_to_docx(md, &fonts, &opts()).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(result.bytes)).unwrap();
        let mut document = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("word/document.xml").unwrap(), &mut document).unwrap();

        assert!(document.contains(r#"w:eastAsia="方正小标宋简体""#));
        assert!(document.contains(r#"<w:sz w:val="44"/>"#), "title is 二号");
        assert!(document.contains("一、会议概况"));
        assert!(document.contains(r#"w:line="579" w:lineRule="exact""#), "body line spacing, 22 lines per page");
        assert!(document.contains(r#"<w:spacing w:val="-5"/>"#), "body condensed to 28 characters per line");
        assert!(document.contains(r#"w:footer="1227""#), "page number 7 mm below the type area");
        assert!(document.contains(r#"w:firstLineChars="200" w:firstLine="640""#));
        assert!(document.contains("会议讨论了 API 接口。"));
        // Table cells are never indented
        assert!(document.contains(r#"w:firstLineChars="0" w:firstLine="0""#));
        assert!(document.contains("<w:tblHeader/>"));
        assert!(document.contains(r#"w:top="2098" w:right="1474" w:bottom="1984" w:left="1588""#));

        let mut odd = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("word/footer1.xml").unwrap(), &mut odd).unwrap();
        assert!(odd.contains(" PAGE ") && odd.contains(r#"<w:jc w:val="right"/>"#) && odd.contains(r#"w:right="280""#));
        let mut even = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("word/footer2.xml").unwrap(), &mut even).unwrap();
        assert!(even.contains(" PAGE ") && even.contains(r#"<w:jc w:val="left"/>"#) && even.contains(r#"w:left="280""#));
        assert!(document.contains(r#"w:type="even" r:id="rIdFooter2""#));
    }

    #[test]
    fn missing_fonts_fall_back_with_warning() {
        let fonts = FontResolver::from_installed(["黑体", "宋体", "仿宋", "楷体"].iter().map(|s| s.to_string()));
        assert_eq!(fonts.resolve("仿宋_GB2312"), "仿宋");
        assert_eq!(fonts.resolve("方正小标宋简体"), "宋体");
        assert_eq!(fonts.resolve("黑体"), "黑体");
        let warnings = fonts.warnings(REQUIRED_FONTS);
        assert!(warnings.iter().any(|w| w.contains("仿宋_GB2312") && w.contains("仿宋")));
    }
}
