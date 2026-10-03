//! Paragraphs to a WordprocessingML package (ECMA-376), the parts Word needs
//! and nothing else: content types, relationships, the document, its styles,
//! the pictures, and the core properties.
//!
//! Every run carries its own font, size, weight and colour, so the document
//! looks right even where a style is missing; the styles are there so Word's
//! navigation pane lists the headings and "Normal" means the body text.

use crate::layout::{Align, Block, Document, Paragraph, PictureBlock, Run, Stats};
use crate::zip::{ZipError, ZipWriter};

/// A finished .docx and what went into it.
#[derive(Debug, Clone, PartialEq)]
pub struct Built {
    pub bytes: Vec<u8>,
    pub stats: Stats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    TooLarge,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::TooLarge => write!(f, "dokumen Word melebihi 4 GB"),
        }
    }
}

impl From<ZipError> for BuildError {
    fn from(_: ZipError) -> Self {
        BuildError::TooLarge
    }
}

/// Points to twentieths of a point, Word's unit for spacing and page size.
fn twips(pt: f32) -> i64 {
    (pt * 20.0).round() as i64
}

/// Points to English Metric Units, DrawingML's unit for picture sizes.
fn emu(pt: f32) -> i64 {
    (pt * 12_700.0).round() as i64
}

/// Escapes text for XML, and drops the control characters XML 1.0 forbids —
/// a PDF can hold them, and one would make Word refuse the whole file.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 => {}
            '\u{FFFE}' | '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

/// A Word font name for a PDF font family. The standard 14 and the fonts
/// PDF producers abbreviate are named as Word knows them; anything else keeps
/// its name with spaces put back between words ("MinionPro" → "Minion Pro"),
/// which is how most families are installed.
pub fn word_font(family: &str) -> String {
    let compact: String = family.chars().filter(|c| !c.is_whitespace()).collect();
    let lower = compact.to_ascii_lowercase();
    let known = [
        ("helvetica", "Arial"),
        ("arial", "Arial"),
        ("timesnewroman", "Times New Roman"),
        ("times", "Times New Roman"),
        ("courier", "Courier New"),
        ("calibri", "Calibri"),
        ("cambria", "Cambria"),
        ("georgia", "Georgia"),
        ("verdana", "Verdana"),
        ("tahoma", "Tahoma"),
        ("segoeui", "Segoe UI"),
        ("garamond", "Garamond"),
        ("symbol", "Symbol"),
        ("zapfdingbats", "Wingdings"),
        ("wingdings", "Wingdings"),
        ("bookantiqua", "Book Antiqua"),
        ("centurygothic", "Century Gothic"),
    ];
    for (prefix, name) in known {
        if lower.starts_with(prefix) {
            return name.to_string();
        }
    }
    if compact.is_empty() {
        return "Calibri".to_string();
    }
    // Spaces back between camel-cased words.
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in compact.chars() {
        if let Some(p) = prev {
            if c.is_uppercase() && p.is_lowercase() {
                out.push(' ');
            }
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

fn run_xml(run: &Run, out: &mut String) {
    let font = escape(&word_font(&run.font));
    let half_points = ((run.size * 2.0).round() as i64).clamp(2, 3276);
    let mut props = format!(
        r#"<w:rFonts w:ascii="{font}" w:hAnsi="{font}" w:cs="{font}" w:eastAsia="{font}"/>"#
    );
    if run.bold {
        props.push_str("<w:b/><w:bCs/>");
    }
    if run.italic {
        props.push_str("<w:i/><w:iCs/>");
    }
    if run.color != [0, 0, 0] {
        props.push_str(&format!(
            r#"<w:color w:val="{:02X}{:02X}{:02X}"/>"#,
            run.color[0], run.color[1], run.color[2]
        ));
    }
    props.push_str(&format!(
        r#"<w:sz w:val="{half_points}"/><w:szCs w:val="{half_points}"/>"#
    ));
    out.push_str("<w:r><w:rPr>");
    out.push_str(&props);
    out.push_str("</w:rPr>");
    // Tabs are their own element; the text between them goes in <w:t>.
    for (k, part) in run.text.split('\t').enumerate() {
        if k > 0 {
            out.push_str("<w:tab/>");
        }
        if !part.is_empty() {
            out.push_str(r#"<w:t xml:space="preserve">"#);
            out.push_str(&escape(part));
            out.push_str("</w:t>");
        }
    }
    out.push_str("</w:r>");
}

fn jc(align: Align) -> Option<&'static str> {
    match align {
        Align::Left => None,
        Align::Center => Some("center"),
        Align::Right => Some("right"),
        Align::Justify => Some("both"),
    }
}

fn paragraph_xml(p: &Paragraph, out: &mut String) {
    out.push_str("<w:p><w:pPr>");
    if let Some(level) = p.heading {
        out.push_str(&format!(r#"<w:pStyle w:val="Heading{level}"/>"#));
    }
    if p.page_break_before {
        out.push_str("<w:pageBreakBefore/>");
    }
    if !p.tabs.is_empty() {
        out.push_str("<w:tabs>");
        for t in &p.tabs {
            out.push_str(&format!(r#"<w:tab w:val="left" w:pos="{}"/>"#, twips(*t)));
        }
        out.push_str("</w:tabs>");
    }
    out.push_str(&format!(
        r#"<w:spacing w:before="{}" w:after="0" w:line="{}" w:lineRule="auto"/>"#,
        twips(p.space_before),
        (p.line_spacing * 240.0).round() as i64
    ));
    if p.indent_left != 0.0 || p.first_line != 0.0 {
        let first = if p.first_line >= 0.0 {
            format!(r#" w:firstLine="{}""#, twips(p.first_line))
        } else {
            format!(r#" w:hanging="{}""#, twips(-p.first_line))
        };
        out.push_str(&format!(
            r#"<w:ind w:left="{}"{first}/>"#,
            twips(p.indent_left)
        ));
    }
    if let Some(v) = jc(p.align) {
        out.push_str(&format!(r#"<w:jc w:val="{v}"/>"#));
    }
    out.push_str("</w:pPr>");
    for run in &p.runs {
        run_xml(run, out);
    }
    out.push_str("</w:p>");
}

fn picture_xml(p: &PictureBlock, rel: &str, id: usize, out: &mut String) {
    let (cx, cy) = (emu(p.width), emu(p.height));
    out.push_str("<w:p><w:pPr>");
    if p.page_break_before {
        out.push_str("<w:pageBreakBefore/>");
    }
    out.push_str(&format!(
        r#"<w:spacing w:before="{}" w:after="0"/>"#,
        twips(p.space_before)
    ));
    if p.indent_left > 0.0 {
        out.push_str(&format!(r#"<w:ind w:left="{}"/>"#, twips(p.indent_left)));
    }
    if let Some(v) = jc(p.align) {
        out.push_str(&format!(r#"<w:jc w:val="{v}"/>"#));
    }
    out.push_str("</w:pPr><w:r><w:drawing>");
    out.push_str(&format!(
        concat!(
            r#"<wp:inline distT="0" distB="0" distL="0" distR="0">"#,
            r#"<wp:extent cx="{cx}" cy="{cy}"/>"#,
            r#"<wp:docPr id="{id}" name="Gambar {id}"/>"#,
            r#"<wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr>"#,
            r#"<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">"#,
            r#"<pic:pic><pic:nvPicPr><pic:cNvPr id="{id}" name="Gambar {id}"/><pic:cNvPicPr/></pic:nvPicPr>"#,
            r#"<pic:blipFill><a:blip r:embed="{rel}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>"#,
            r#"<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>"#,
            r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>"#,
            r#"</a:graphicData></a:graphic></wp:inline>"#
        ),
        cx = cx,
        cy = cy,
        id = id,
        rel = rel
    ));
    out.push_str("</w:drawing></w:r></w:p>");
}

fn extension(mime: &str) -> &'static str {
    if mime == "image/jpeg" {
        "jpeg"
    } else {
        "png"
    }
}

fn document_xml(doc: &Document) -> String {
    let mut body = String::new();
    for block in &doc.blocks {
        match block {
            Block::Paragraph(p) => paragraph_xml(p, &mut body),
            Block::Picture(p) => {
                let rel = format!("rIdImg{}", p.picture + 1);
                picture_xml(p, &rel, p.picture + 1, &mut body);
            }
        }
    }
    let [left, top, right, bottom] = doc.margins;
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" "#,
            r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
            r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
            r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
            r#"xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">"#,
            "<w:body>{body}",
            r#"<w:sectPr><w:pgSz w:w="{w}" w:h="{h}"{orient}/>"#,
            r#"<w:pgMar w:top="{top}" w:right="{right}" w:bottom="{bottom}" w:left="{left}" w:header="0" w:footer="0" w:gutter="0"/>"#,
            "</w:sectPr></w:body></w:document>"
        ),
        body = body,
        w = twips(doc.page_width),
        h = twips(doc.page_height),
        orient = if doc.page_width > doc.page_height {
            r#" w:orient="landscape""#
        } else {
            ""
        },
        top = twips(top),
        right = twips(right),
        bottom = twips(bottom),
        left = twips(left),
    )
}

fn styles_xml(doc: &Document) -> String {
    let font = escape(&word_font(&doc.body_font));
    let body = ((doc.body_size * 2.0).round() as i64).clamp(2, 3276);
    let heading = |level: u8, scale: f32| {
        format!(
            concat!(
                r#"<w:style w:type="paragraph" w:styleId="Heading{l}">"#,
                r#"<w:name w:val="heading {l}"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/>"#,
                r#"<w:pPr><w:keepNext/><w:outlineLvl w:val="{lvl}"/></w:pPr>"#,
                r#"<w:rPr><w:b/><w:sz w:val="{sz}"/></w:rPr></w:style>"#
            ),
            l = level,
            lvl = level - 1,
            sz = (body as f32 * scale).round() as i64
        )
    };
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
            r#"<w:docDefaults><w:rPrDefault><w:rPr>"#,
            r#"<w:rFonts w:ascii="{font}" w:hAnsi="{font}" w:cs="{font}" w:eastAsia="{font}"/>"#,
            r#"<w:sz w:val="{body}"/><w:szCs w:val="{body}"/><w:lang w:val="id-ID"/>"#,
            r#"</w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>"#,
            r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>"#,
            "{h1}{h2}{h3}",
            "</w:styles>"
        ),
        font = font,
        body = body,
        h1 = heading(1, 2.0),
        h2 = heading(2, 1.6),
        h3 = heading(3, 1.3),
    )
}

fn content_types(doc: &Document) -> String {
    let jpeg = doc.pictures.iter().any(|p| p.mime == "image/jpeg");
    let png = doc.pictures.iter().any(|p| p.mime != "image/jpeg");
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
            r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
            r#"<Default Extension="xml" ContentType="application/xml"/>"#,
            "{jpeg}{png}",
            r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
            r#"<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>"#,
            r#"<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>"#,
            r#"<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>"#,
            "</Types>"
        ),
        jpeg = if jpeg {
            r#"<Default Extension="jpeg" ContentType="image/jpeg"/>"#
        } else {
            ""
        },
        png = if png {
            r#"<Default Extension="png" ContentType="image/png"/>"#
        } else {
            ""
        },
    )
}

const ROOT_RELS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>"#,
    r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>"#,
    r#"<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>"#,
    "</Relationships>"
);

fn document_rels(doc: &Document) -> String {
    let mut out = String::from(concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        r#"<Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#
    ));
    for (k, p) in doc.pictures.iter().enumerate() {
        out.push_str(&format!(
            r#"<Relationship Id="rIdImg{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image{n}.{ext}"/>"#,
            n = k + 1,
            ext = extension(&p.mime)
        ));
    }
    out.push_str("</Relationships>");
    out
}

fn core_xml(title: &str) -> String {
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" "#,
            r#"xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" "#,
            r#"xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#,
            "<dc:title>{title}</dc:title>",
            "</cp:coreProperties>"
        ),
        title = escape(title)
    )
}

const APP_XML: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties">"#,
    "<Application>PDF Studio Izul</Application>",
    "</Properties>"
);

/// Writes `doc` as a .docx. `title` goes into the document's properties.
pub fn build(doc: &Document, title: &str) -> Result<Built, BuildError> {
    let mut z = ZipWriter::default();
    z.add("[Content_Types].xml", content_types(doc).as_bytes(), true)?;
    z.add("_rels/.rels", ROOT_RELS.as_bytes(), true)?;
    z.add("word/document.xml", document_xml(doc).as_bytes(), true)?;
    z.add("word/styles.xml", styles_xml(doc).as_bytes(), true)?;
    z.add(
        "word/_rels/document.xml.rels",
        document_rels(doc).as_bytes(),
        true,
    )?;
    for (k, p) in doc.pictures.iter().enumerate() {
        z.add(
            &format!("word/media/image{}.{}", k + 1, extension(&p.mime)),
            &p.bytes,
            false,
        )?;
    }
    z.add("docProps/core.xml", core_xml(title).as_bytes(), true)?;
    z.add("docProps/app.xml", APP_XML.as_bytes(), true)?;
    Ok(Built {
        bytes: z.finish()?,
        stats: doc.stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_fonts_the_way_word_does() {
        assert_eq!(word_font("Helvetica"), "Arial");
        assert_eq!(word_font("TimesNewRoman"), "Times New Roman");
        assert_eq!(word_font("Times"), "Times New Roman");
        assert_eq!(word_font("Courier"), "Courier New");
        assert_eq!(word_font("MinionPro"), "Minion Pro");
        assert_eq!(word_font("SegoeUI"), "Segoe UI");
        assert_eq!(word_font(""), "Calibri");
    }

    #[test]
    fn escapes_markup_and_drops_what_xml_forbids() {
        assert_eq!(escape("a < b & \"c\""), "a &lt; b &amp; &quot;c&quot;");
        assert_eq!(escape("x\u{1}y\u{FFFF}"), "xy");
        assert_eq!(escape("tab\there"), "tab\there");
    }

    #[test]
    fn a_tab_in_a_run_is_a_tab_element() {
        let mut out = String::new();
        run_xml(
            &Run {
                text: "Nama\tNilai".into(),
                bold: true,
                italic: false,
                size: 11.0,
                color: [0, 0, 0],
                font: "Helvetica".into(),
            },
            &mut out,
        );
        assert!(out.contains("<w:t xml:space=\"preserve\">Nama</w:t><w:tab/><w:t xml:space=\"preserve\">Nilai</w:t>"), "{out}");
        assert!(
            out.contains("<w:b/>") && out.contains("w:val=\"22\""),
            "{out}"
        );
    }
}
