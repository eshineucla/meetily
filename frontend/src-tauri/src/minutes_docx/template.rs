//! Export into the user's own Word template (.dotx / .docx).
//!
//! Many offices keep a style-based official-document template: page setup, page numbers and
//! fonts live in the template, and the text only applies its styles (标题, 标题 1–3, 正文).
//! Here the template package is kept as it is and only the body is replaced with paragraphs
//! that reference those styles by name, so the exported file keeps working with the template's
//! own style pane, multilevel numbering and footers. Templates are chosen by the user and never
//! shipped with the app.

use super::fonts::FontResolver;
use super::normalize::normalize_text;
use super::{escape_xml, parse_markdown, strip_heading_number, Block, ExportOptions, ExportResult};
use regex::Regex;
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::sync::LazyLock;

static STYLE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<w:style\b([^>]*)>(.*?)</w:style>").unwrap());
static ATTR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"w:(type|styleId|default)="([^"]*)""#).unwrap());
static NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<w:name w:val="([^"]*)""#).unwrap());
static BASED_ON_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<w:basedOn w:val="([^"]*)""#).unwrap());
static EAST_ASIA_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"w:eastAsia="([^"]+)""#).unwrap());
static DEFAULTS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<w:docDefaults>.*?</w:docDefaults>").unwrap());
static REL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<Relationship\b[^>]*>"#).unwrap());
static ATTACHED_TEMPLATE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<w:attachedTemplate\b[^>]*/>").unwrap());
static PERSONAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)(<(dc:creator|cp:lastModifiedBy|Company|Manager)>).*?(</(?:dc:creator|cp:lastModifiedBy|Company|Manager)>)").unwrap()
});

const MAIN_DOCUMENT: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const TEMPLATE_MAIN: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml";

/// Table text is 小四, as in the built-in layout
const TABLE_SIZE_HALF_POINTS: u32 = 24;

#[derive(Debug, Clone)]
struct StyleInfo {
    id: String,
    name: String,
    body: String,
    based_on: Option<String>,
}

/// The paragraph styles of a template, looked up by role
struct TemplateStyles {
    styles: Vec<StyleInfo>,
    default_id: Option<String>,
}

impl TemplateStyles {
    fn parse(styles_xml: &str) -> Self {
        let mut styles = Vec::new();
        let mut default_id = None;
        for m in STYLE_RE.captures_iter(styles_xml) {
            let attrs: HashMap<&str, &str> =
                ATTR_RE.captures_iter(&m[1]).map(|a| (a.get(1).unwrap().as_str(), a.get(2).unwrap().as_str())).collect();
            if attrs.get("type") != Some(&"paragraph") {
                continue;
            }
            let Some(id) = attrs.get("styleId") else { continue };
            let body = m[2].to_string();
            if matches!(attrs.get("default"), Some(&"1") | Some(&"true")) {
                default_id = Some(id.to_string());
            }
            styles.push(StyleInfo {
                id: id.to_string(),
                name: NAME_RE.captures(&body).map(|n| n[1].to_string()).unwrap_or_default(),
                based_on: BASED_ON_RE.captures(&body).map(|b| b[1].to_string()),
                body,
            });
        }
        Self { styles, default_id }
    }

    /// Style id for the first of `names` the template defines (Word stores built-in names in English)
    fn find(&self, names: &[&str]) -> Option<&StyleInfo> {
        names.iter().find_map(|name| self.styles.iter().find(|s| s.name.eq_ignore_ascii_case(name)))
    }

    fn by_id(&self, id: &str) -> Option<&StyleInfo> {
        self.styles.iter().find(|s| s.id == id)
    }

    /// Whether the style, or a style it is based on, carries automatic numbering
    fn is_numbered(&self, style: &StyleInfo) -> bool {
        let mut current = Some(style);
        for _ in 0..10 {
            let Some(s) = current else { return false };
            if s.body.contains("<w:numPr>") {
                return true;
            }
            current = s.based_on.as_deref().and_then(|id| self.by_id(id));
        }
        false
    }

    /// East Asian fonts named by the styles the export uses, and by the document defaults
    fn fonts(&self, styles_xml: &str, used: &[&StyleInfo]) -> Vec<String> {
        let mut fonts: Vec<String> = Vec::new();
        let defaults = DEFAULTS_RE.find(styles_xml).map(|m| m.as_str()).unwrap_or("");
        let mut sources = vec![defaults.to_string()];
        sources.extend(used.iter().map(|s| s.body.clone()));
        for source in sources {
            for m in EAST_ASIA_RE.captures_iter(&source) {
                let font = m[1].to_string();
                if !fonts.contains(&font) {
                    fonts.push(font);
                }
            }
        }
        fonts
    }
}

struct Roles<'a> {
    title: Option<&'a StyleInfo>,
    headings: [Option<&'a StyleInfo>; 3],
    no_indent: Option<&'a StyleInfo>,
}

fn find_roles(styles: &TemplateStyles) -> Roles<'_> {
    Roles {
        title: styles.find(&["Title", "标题", "公文标题"]),
        headings: [
            styles.find(&["heading 1", "标题 1"]),
            styles.find(&["heading 2", "标题 2"]),
            styles.find(&["heading 3", "标题 3"]),
        ],
        no_indent: styles.find(&["正文-无缩进", "正文无缩进", "No Indent"]),
    }
}

struct Writer<'a> {
    styles: &'a TemplateStyles,
    roles: Roles<'a>,
    options: &'a ExportOptions,
    fonts: &'a FontResolver,
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

    /// A paragraph in the given style; `extra_ppr` and `rpr` override it only where the export must
    fn paragraph_xml(&self, style: Option<&StyleInfo>, extra_ppr: &str, rpr: &str, text: &str) -> String {
        let style_ref = style
            .filter(|s| Some(&s.id) != self.styles.default_id.as_ref())
            .map(|s| format!(r#"<w:pStyle w:val="{}"/>"#, escape_xml(&s.id)))
            .unwrap_or_default();
        let ppr = if style_ref.is_empty() && extra_ppr.is_empty() { String::new() } else { format!("<w:pPr>{style_ref}{extra_ppr}</w:pPr>") };
        let run = if text.is_empty() {
            String::new()
        } else {
            format!(r#"<w:r>{rpr}<w:t xml:space="preserve">{}</w:t></w:r>"#, escape_xml(&self.text(text)))
        };
        format!("<w:p>{ppr}{run}</w:p>")
    }

    fn paragraph(&mut self, style: Option<&StyleInfo>, text: &str) {
        let xml = self.paragraph_xml(style, "", "", text);
        self.body.push_str(&xml);
    }

    /// Body text without first-line indent: the template's no-indent style, or body with the indent cleared
    fn unindented(&mut self, text: &str, rpr: &str) -> String {
        match self.roles.no_indent {
            Some(style) => self.paragraph_xml(Some(style), "", rpr, text),
            None => self.paragraph_xml(None, r#"<w:ind w:firstLineChars="0" w:firstLine="0"/>"#, rpr, text),
        }
    }

    fn heading(&mut self, level: u8, text: &str) {
        let style = self.roles.headings[(level as usize).clamp(1, 3) - 1];
        // A numbered heading style writes 一、 itself; the text must not repeat it
        let text = match style {
            Some(s) if self.styles.is_numbered(s) => strip_heading_number(text),
            _ => text.to_string(),
        };
        self.paragraph(style, &text);
    }

    fn table(&mut self, rows: &[Vec<String>]) {
        let cols = rows.first().map(Vec::len).unwrap_or(0);
        if cols == 0 {
            return;
        }
        let col_width = 8844 / cols as u32;
        let mut xml = String::from(r#"<w:tbl><w:tblPr><w:tblW w:w="5000" w:type="pct"/><w:tblInd w:w="0" w:type="dxa"/><w:tblBorders>"#);
        for edge in ["top", "left", "bottom", "right", "insideH", "insideV"] {
            xml.push_str(&format!(r#"<w:{edge} w:val="single" w:sz="4" w:space="0" w:color="000000"/>"#));
        }
        xml.push_str(r#"</w:tblBorders><w:tblLayout w:type="fixed"/><w:tblCellMar><w:top w:w="28" w:type="dxa"/><w:start w:w="85" w:type="dxa"/><w:bottom w:w="28" w:type="dxa"/><w:end w:w="85" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid>"#);
        for _ in 0..cols {
            xml.push_str(&format!(r#"<w:gridCol w:w="{col_width}"/>"#));
        }
        xml.push_str("</w:tblGrid>");
        let head_font = escape_xml(&self.fonts.resolve("黑体"));
        for (r, row) in rows.iter().enumerate() {
            let rpr = if r == 0 {
                format!(r#"<w:rPr><w:rFonts w:eastAsia="{head_font}"/><w:b/><w:sz w:val="{TABLE_SIZE_HALF_POINTS}"/><w:szCs w:val="{TABLE_SIZE_HALF_POINTS}"/></w:rPr>"#)
            } else {
                format!(r#"<w:rPr><w:sz w:val="{TABLE_SIZE_HALF_POINTS}"/><w:szCs w:val="{TABLE_SIZE_HALF_POINTS}"/></w:rPr>"#)
            };
            xml.push_str("<w:tr>");
            if r == 0 && rows.len() > 1 {
                xml.push_str("<w:trPr><w:tblHeader/></w:trPr>");
            }
            for cell in row {
                xml.push_str(&format!(r#"<w:tc><w:tcPr><w:tcW w:w="{col_width}" w:type="dxa"/></w:tcPr>"#));
                // Table rows keep single spacing even when the template snaps body text to a grid
                let p = self.unindented(cell, &rpr).replacen("<w:pPr>", r#"<w:pPr><w:snapToGrid w:val="0"/>"#, 1);
                let p = if p.contains("<w:pPr>") { p } else { p.replacen("<w:p>", r#"<w:p><w:pPr><w:snapToGrid w:val="0"/></w:pPr>"#, 1) };
                xml.push_str(&p);
                xml.push_str("</w:tc>");
            }
            xml.push_str("</w:tr>");
        }
        xml.push_str("</w:tbl>");
        self.body.push_str(&xml);
        // Two tables with no paragraph between them merge into one in Word
        self.paragraph(None, "");
    }
}

fn render_body(blocks: &[Block], styles: &TemplateStyles, options: &ExportOptions, fonts: &FontResolver) -> String {
    let mut writer = Writer { styles, roles: find_roles(styles), options, fonts, body: String::new() };
    for block in blocks {
        match block {
            Block::Title(text) => {
                let title = writer.roles.title;
                writer.paragraph(title, text);
            }
            Block::Heading(level, text) => writer.heading(*level, text),
            Block::Paragraph(text) | Block::Quote(text) => writer.paragraph(None, text),
            Block::ListItem(marker, text) => {
                let xml = writer.unindented(&format!("{} {}", marker, text), "");
                writer.body.push_str(&xml);
            }
            Block::Table(rows) => writer.table(rows),
            Block::Rule => writer.paragraph(None, ""),
        }
    }
    writer.body
}

/// Replace everything in <w:body> except its final section properties (page setup, headers, footers)
fn replace_body(document_xml: &str, body: &str) -> Result<String, String> {
    let open = document_xml.find("<w:body>").ok_or("模板里找不到正文（w:body）")?;
    let start = open + "<w:body>".len();
    let end = document_xml.rfind("</w:body>").ok_or("模板里找不到正文结尾（/w:body）")?;
    let inner = &document_xml[start..end];
    // The body-level sectPr is the last element; section breaks inside paragraphs come before it
    let sect = inner.rfind("<w:sectPr").map(|i| &inner[i..]).filter(|s| s.trim_end().ends_with("</w:sectPr>") || s.trim_end().ends_with("/>"));
    Ok(format!("{}{}{}{}", &document_xml[..start], body, sect.unwrap_or(""), &document_xml[end..]))
}

fn rel_target(rels_xml: &str, type_suffix: &str) -> Option<String> {
    REL_RE.find_iter(rels_xml).map(|m| m.as_str()).find(|r| r.contains(&format!("/{}\"", type_suffix))).and_then(|r| {
        let start = r.find("Target=\"")? + "Target=\"".len();
        let end = r[start..].find('"')? + start;
        Some(r[start..end].trim_start_matches('/').to_string())
    })
}

fn read_entry(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<String, String> {
    let mut file = archive.by_name(name).map_err(|_| format!("模板里缺少 {}", name))?;
    let mut text = String::new();
    file.read_to_string(&mut text).map_err(|e| format!("读取模板 {} 失败：{}", name, e))?;
    Ok(text)
}

/// Fill the template with the minutes. Fails with a readable message when the file is not a usable Word template.
pub fn render_with_template(markdown: &str, template: &[u8], fonts: &FontResolver, options: &ExportOptions) -> Result<ExportResult, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(template)).map_err(|_| "模板不是有效的 Word 文件（.docx / .dotx）".to_string())?;

    let content_types = read_entry(&mut archive, "[Content_Types].xml")?;
    if content_types.contains("macroEnabled") {
        return Err("不支持带宏的模板，请在 Word 里另存为 .dotx 后再选择".to_string());
    }
    let root_rels = read_entry(&mut archive, "_rels/.rels")?;
    let document_path = rel_target(&root_rels, "officeDocument").unwrap_or_else(|| "word/document.xml".to_string());
    let (dir, file) = document_path.rsplit_once('/').unwrap_or(("", document_path.as_str()));
    let document_rels_path = if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") };
    let document_rels = read_entry(&mut archive, &document_rels_path).unwrap_or_default();
    let styles_path = rel_target(&document_rels, "styles")
        .map(|t| if dir.is_empty() { t } else { format!("{dir}/{t}") })
        .unwrap_or_else(|| "word/styles.xml".to_string());
    let settings_path = rel_target(&document_rels, "settings").map(|t| if dir.is_empty() { t } else { format!("{dir}/{t}") });

    let styles_xml = read_entry(&mut archive, &styles_path)?;
    let document_xml = read_entry(&mut archive, &document_path)?;
    let styles = TemplateStyles::parse(&styles_xml);

    let (blocks, mut warnings) = parse_markdown(markdown, options);
    let roles = find_roles(&styles);
    if roles.title.is_none() {
        warnings.push("模板里没有“标题”样式，纪要标题按正文排版".to_string());
    }
    let deepest = blocks.iter().filter_map(|b| if let Block::Heading(l, _) = b { Some(*l) } else { None }).max().unwrap_or(0);
    for level in 1..=deepest {
        if roles.headings[level as usize - 1].is_none() {
            warnings.push(format!("模板里没有“标题 {}”样式，这一级标题按正文排版", level));
        }
    }
    let mut used: Vec<&StyleInfo> = roles.headings.iter().flatten().copied().collect();
    used.extend(roles.title);
    used.extend(styles.default_id.as_deref().and_then(|id| styles.by_id(id)));
    if fonts.is_available() {
        for font in styles.fonts(&styles_xml, &used) {
            if !fonts.is_installed_name(&font) {
                warnings.push(format!("本机没有安装模板用到的字体“{}”，Word 会用其他字体显示", font));
            }
        }
    }

    let body = render_body(&blocks, &styles, options, fonts);
    let document_xml = replace_body(&document_xml, &body)?;
    let content_types = content_types.replace(TEMPLATE_MAIN, MAIN_DOCUMENT);

    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let file_options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        if entry.is_dir() {
            continue;
        }
        let replaced = if name == "[Content_Types].xml" {
            Some(content_types.clone())
        } else if name == document_path {
            Some(document_xml.clone())
        } else if Some(&name) == settings_path.as_ref() {
            // The template's own path on the author's machine would be kept otherwise
            let mut text = String::new();
            entry.read_to_string(&mut text).map_err(|e| e.to_string())?;
            Some(ATTACHED_TEMPLATE_RE.replace_all(&text, "").into_owned())
        } else if name == "docProps/core.xml" || name == "docProps/app.xml" {
            // The template author's name and company are not the minutes' author
            let mut text = String::new();
            entry.read_to_string(&mut text).map_err(|e| e.to_string())?;
            Some(PERSONAL_RE.replace_all(&text, "$1$3").into_owned())
        } else {
            None
        };
        match replaced {
            Some(text) => {
                zip.start_file(name, file_options).map_err(|e| e.to_string())?;
                zip.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
            }
            None => {
                drop(entry);
                let entry = archive.by_index_raw(i).map_err(|e| e.to_string())?;
                zip.raw_copy_file(entry).map_err(|e| e.to_string())?;
            }
        }
    }
    let bytes = zip.finish().map_err(|e| e.to_string())?.into_inner();
    Ok(ExportResult { bytes, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLES: &str = r#"<w:styles><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:eastAsia="仿宋"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="a3"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="ad"><w:name w:val="Title"/><w:basedOn w:val="a3"/><w:rPr><w:rFonts w:eastAsia="方正小标宋简体"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="1"><w:name w:val="heading 1"/><w:basedOn w:val="a3"/><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr></w:pPr></w:style><w:style w:type="paragraph" w:styleId="21"><w:name w:val="heading 2"/><w:basedOn w:val="a3"/></w:style><w:style w:type="paragraph" w:customStyle="1" w:styleId="-c"><w:name w:val="正文-无缩进"/><w:basedOn w:val="a3"/></w:style><w:style w:type="character" w:styleId="x"><w:name w:val="heading 3"/></w:style></w:styles>"#;

    fn template() -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        let parts = [
            ("[Content_Types].xml", format!(r#"<Types><Override PartName="/word/document.xml" ContentType="{TEMPLATE_MAIN}"/></Types>"#)),
            ("_rels/.rels", r#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string()),
            ("word/_rels/document.xml.rels", r#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/></Relationships>"#.to_string()),
            ("word/document.xml", r#"<w:document><w:body><w:p><w:r><w:t>使用说明</w:t></w:r></w:p><w:p><w:pPr><w:sectPr><w:pgSz w:w="1"/></w:sectPr></w:pPr></w:p><w:sectPr><w:footerReference w:type="default" r:id="rId9"/><w:titlePg/></w:sectPr></w:body></w:document>"#.to_string()),
            ("word/styles.xml", STYLES.to_string()),
            ("word/settings.xml", r#"<w:settings><w:attachedTemplate r:id="rId1"/><w:evenAndOddHeaders/></w:settings>"#.to_string()),
            ("docProps/core.xml", "<cp:coreProperties><dc:creator>someone</dc:creator><cp:lastModifiedBy>someone</cp:lastModifiedBy></cp:coreProperties>".to_string()),
        ];
        for (name, content) in parts {
            zip.start_file(name, o).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn entry(bytes: &[u8], name: &str) -> String {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        read_entry(&mut archive, name).unwrap()
    }

    #[test]
    fn styles_are_found_by_name_and_numbering_is_inherited() {
        let styles = TemplateStyles::parse(STYLES);
        let roles = find_roles(&styles);
        assert_eq!(roles.title.unwrap().id, "ad");
        assert_eq!(roles.headings[0].unwrap().id, "1");
        assert!(styles.is_numbered(roles.headings[0].unwrap()));
        assert!(!styles.is_numbered(roles.headings[1].unwrap()));
        // A character style named "heading 3" is not a paragraph style
        assert!(roles.headings[2].is_none());
        assert_eq!(roles.no_indent.unwrap().id, "-c");
    }

    #[test]
    fn body_is_replaced_and_template_parts_kept() {
        let md = "# 预算会纪要\n\n## 一、会议概况\n正文内容\n\n### （一）预算\n- 要点\n\n| 序号 | 事项 |\n| --- | --- |\n| 1 | 编表 |\n";
        let result = render_with_template(md, &template(), &FontResolver::unknown(), &ExportOptions::default()).unwrap();
        let doc = entry(&result.bytes, "word/document.xml");
        assert!(!doc.contains("使用说明"), "template instructions must be removed");
        assert!(!doc.contains(r#"<w:pgSz w:w="1"/>"#), "section breaks inside the old body go too");
        assert!(doc.contains(r#"<w:footerReference w:type="default" r:id="rId9"/><w:titlePg/></w:sectPr></w:body>"#));
        assert!(doc.contains(r#"<w:pStyle w:val="ad"/></w:pPr><w:r><w:t xml:space="preserve">预算会纪要"#));
        // Numbered heading style: the number is left to Word
        assert!(doc.contains(r#"<w:pStyle w:val="1"/></w:pPr><w:r><w:t xml:space="preserve">会议概况"#));
        // Unnumbered heading style keeps the written number
        assert!(doc.contains("（一）预算"));
        // Body uses the default style without naming it
        assert!(doc.contains(r#"<w:p><w:r><w:t xml:space="preserve">正文内容"#));
        assert!(doc.contains(r#"<w:pStyle w:val="-c"/></w:pPr><w:r><w:t xml:space="preserve">○ 要点"#));
        assert!(doc.contains("<w:tblHeader/>"));

        assert!(entry(&result.bytes, "[Content_Types].xml").contains(MAIN_DOCUMENT));
        assert!(!entry(&result.bytes, "word/settings.xml").contains("attachedTemplate"));
        assert!(!entry(&result.bytes, "docProps/core.xml").contains("someone"));
        assert_eq!(entry(&result.bytes, "word/styles.xml"), STYLES);
    }

    #[test]
    fn missing_styles_and_fonts_are_reported() {
        let md = "# 纪要\n\n## 概况\n\n### 细节\n\n#### 更细\n";
        let fonts = FontResolver::from_installed(["仿宋".to_string()]);
        let result = render_with_template(md, &template(), &fonts, &ExportOptions::default()).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("标题 3")));
        assert!(result.warnings.iter().any(|w| w.contains("方正小标宋简体")));
        assert!(!result.warnings.iter().any(|w| w.contains("“仿宋”")));
    }

    #[test]
    fn non_word_files_are_rejected() {
        assert!(render_with_template("# x", b"not a zip", &FontResolver::unknown(), &ExportOptions::default()).is_err());
    }
}
