//! Bounded extraction of Office XML archives, including aggregate inflated size and output.
use std::io::{Cursor, Read};
use roxmltree::{Document, Node, ParsingOptions};

const PART_BYTES: u64 = 50 * 1024 * 1024;
const MAX_ENTRIES: usize = 2048;
const MAX_NODES: u32 = 250_000;
const OUTPUT_BYTES: usize = crate::MAX_CHARS * 4;

struct Zip<'a> { archive: zip::ZipArchive<Cursor<&'a [u8]>>, remaining: u64 }

fn open(bytes: &[u8]) -> Option<Zip<'_>> {
    if bytes.len() as u64 > PART_BYTES { return None; }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    if archive.len() > MAX_ENTRIES { return None; }
    let mut total = 0u64;
    for index in 0..archive.len() {
        total = total.checked_add(archive.by_index_raw(index).ok()?.size())?;
        if total > PART_BYTES { return None; }
    }
    Some(Zip { archive, remaining: PART_BYTES })
}

fn part(zip: &mut Zip<'_>, name: &str) -> Option<String> {
    if zip.remaining == 0 { return None; }
    let file = zip.archive.by_name(name).ok()?;
    let mut bytes = Vec::new();
    let result = file.take(zip.remaining + 1).read_to_end(&mut bytes);
    let exceeded = bytes.len() as u64 > zip.remaining;
    zip.remaining = zip.remaining.saturating_sub(bytes.len() as u64);
    // A broken stream can consume work without yielding bytes. Stop this archive.
    if result.is_err() { zip.remaining = 0; return None; }
    if exceeded { return None; }
    String::from_utf8(bytes).ok()
}

fn numbered(zip: &mut Zip<'_>, prefix: &str) -> Vec<String> {
    let mut names: Vec<(u32, String)> = zip.archive.file_names().filter_map(|n| {
        let n = n.ok()?;
        Some((n.strip_prefix(prefix)?.strip_suffix(".xml")?.parse().ok()?, n.to_string()))
    }).collect();
    names.sort();
    names.into_iter().filter_map(|(_, name)| part(zip, &name)).collect()
}

fn parse(xml: &str) -> Option<Document<'_>> {
    Document::parse_with_options(xml, ParsingOptions { nodes_limit: MAX_NODES, ..Default::default() }).ok()
}

#[derive(Default)]
struct Text(String);
impl Text {
    fn push(&mut self, value: &str) {
        let mut end = value.len().min(OUTPUT_BYTES - self.0.len());
        while !value.is_char_boundary(end) { end -= 1; }
        self.0.push_str(&value[..end]);
    }
    fn full(&self) -> bool { self.0.len() >= OUTPUT_BYTES }
}

fn words(xml: &str) -> String {
    let Some(doc) = parse(xml) else { return String::new() };
    let mut out = Text::default();
    walk(doc.root(), &mut out);
    out.0
}

/// Explicit traversal avoids using the native stack for attacker-controlled XML depth.
fn walk(node: Node<'_, '_>, out: &mut Text) {
    let mut stack = vec![(node, false)];
    while let Some((node, closing)) = stack.pop() {
        if out.full() { break; }
        let name = node.tag_name().name();
        if closing {
            match name { "p" | "tr" => out.push("\n"), "tc" => out.push("\t"), _ => {} }
            continue;
        }
        stack.push((node, true));
        match name {
            "t" => out.push(node.text().unwrap_or_default()),
            "tab" => out.push("\t"),
            "br" | "cr" => out.push("\n"),
            _ => stack.extend(node.children().rev().map(|child| (child, false))),
        }
    }
}

pub fn word(bytes: &[u8]) -> Option<String> {
    let mut zip = open(bytes)?;
    let body = part(&mut zip, "word/document.xml")?;
    let mut out = Text::default();
    out.push(&words(&body));
    out.push("\n\n");
    if !out.full() {
        if let Some(notes) = part(&mut zip, "word/footnotes.xml") { out.push(&words(&notes)); }
    }
    Some(out.0)
}

pub fn slides(bytes: &[u8]) -> Option<String> {
    let mut zip = open(bytes)?;
    let mut out = Text::default();
    for (index, xml) in numbered(&mut zip, "ppt/slides/slide").iter().enumerate() {
        if index > 0 { out.push("\n\n"); }
        out.push(&words(xml));
        if out.full() { break; }
    }
    Some(out.0)
}

pub fn sheet(bytes: &[u8]) -> Option<String> {
    let mut zip = open(bytes)?;
    let shared: Vec<String> = part(&mut zip, "xl/sharedStrings.xml").and_then(|xml| {
        let doc = parse(&xml)?;
        Some(doc.root_element().children().filter(|n| n.has_tag_name("si")).map(|si| {
            let mut text = Text::default();
            for value in si.descendants().filter(|n| n.has_tag_name("t")).filter_map(|n| n.text()) { text.push(value); }
            text.0
        }).collect())
    }).unwrap_or_default();
    let mut out = Text::default();
    for (index, xml) in numbered(&mut zip, "xl/worksheets/sheet").iter().enumerate() {
        if index > 0 { out.push("\n\n"); }
        out.push(&rows(xml, &shared));
        if out.full() { break; }
    }
    Some(out.0)
}

fn rows(xml: &str, shared: &[String]) -> String {
    let Some(doc) = parse(xml) else { return String::new() };
    let mut out = Text::default();
    for row in doc.descendants().filter(|n| n.has_tag_name("row")) {
        let mut line = Text::default();
        for (index, cell) in row.children().filter(|c| c.has_tag_name("c")).enumerate() {
            if index > 0 { line.push("\t"); }
            let value = cell.children().find(|v| v.has_tag_name("v")).and_then(|v| v.text()).unwrap_or_default();
            match cell.attribute("t") {
                Some("s") => line.push(value.parse::<usize>().ok().and_then(|i| shared.get(i)).map_or("", String::as_str)),
                Some("inlineStr") => {
                    for text in cell.descendants().filter(|n| n.has_tag_name("t")).filter_map(|n| n.text()) { line.push(text); }
                }
                _ => line.push(value),
            }
            if line.full() { break; }
        }
        let line = line.0.trim_end();
        if !line.is_empty() {
            if !out.0.is_empty() { out.push("\n"); }
            out.push(line);
        }
        if out.full() { break; }
    }
    out.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::zip;

    const P: &str = r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;
    const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

    #[test]
    fn a_word_document_reads_as_paragraphs_with_its_tables_and_footnotes() {
        let body = format!(
            r#"<w:document {W}><w:body>
                <w:p><w:r><w:t>Dear Maya,</w:t></w:r></w:p>
                <w:p><w:r><w:t xml:space="preserve">Your start date is </w:t></w:r><w:r><w:t>1 Dec &amp; pay is monthly.</w:t></w:r></w:p>
                <w:p><w:r><w:t>Name</w:t><w:tab/><w:t>Role</w:t><w:br/><w:t>next line</w:t></w:r></w:p>
                <w:tbl><w:tr><w:tc><w:p><w:r><w:t>Salary</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>90,000</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
            </w:body></w:document>"#
        );
        let notes = format!(r#"<w:footnotes {W}><w:footnote><w:p><w:r><w:t>Before tax.</w:t></w:r></w:p></w:footnote></w:footnotes>"#);
        let text = word(&zip(&[("word/document.xml", &body), ("word/footnotes.xml", &notes)])).unwrap();
        let text = crate::tidy(&text);
        assert_eq!(
            text,
            "Dear Maya,\nYour start date is 1 Dec & pay is monthly.\nName\tRole\nnext line\nSalary\n\t90,000\n\nBefore tax."
        );
        // No footnotes is fine; no body isn't a Word file.
        assert_eq!(crate::tidy(&word(&zip(&[("word/document.xml", &body)])).unwrap()).lines().next(), Some("Dear Maya,"));
        assert_eq!(word(&zip(&[("other.xml", "<x/>")])), None);
        assert_eq!(word(b"PK nope"), None);
    }

    #[test]
    fn broken_xml_reads_as_no_words() {
        assert_eq!(words("<w:p><unclosed>"), "");
        assert_eq!(rows("<row>", &[]), "");
        assert_eq!(crate::tidy(&word(&zip(&[("word/document.xml", "<<")])).unwrap()), "");
    }

    #[test]
    fn slides_are_read_in_order() {
        let slide = |t: &str| format!("<p:sld {P}><p:txBody><a:p><a:r><a:t>{t}</a:t></a:r></a:p><a:p><a:r><a:t>more</a:t></a:r></a:p></p:txBody></p:sld>");
        let (one, two, ten) = (slide("Intro"), slide("Plan"), slide("Thanks"));
        let deck = zip(&[
            ("ppt/slides/slide10.xml", &ten),
            ("ppt/slides/slide2.xml", &two),
            ("ppt/slides/slide1.xml", &one),
            ("ppt/slides/_rels/slide1.xml.rels", "<x/>"),
            ("ppt/slides/slideLayout.xml", "<x/>"),
        ]);
        assert_eq!(crate::tidy(&slides(&deck).unwrap()), "Intro\nmore\n\nPlan\nmore\n\nThanks\nmore");
        assert_eq!(slides(b"no"), None);
    }

    #[test]
    fn a_workbook_reads_each_sheet_as_rows_of_cells() {
        let shared = r#"<sst><si><t>Item</t></si><si><r><t>Fl</t></r><r><t>ights</t></r></si><si><t>Hotel</t></si></sst>"#;
        let first = r#"<worksheet><sheetData>
            <row><c t="s"><v>0</v></c><c t="inlineStr"><is><t>Cost</t></is></c></row>
            <row><c t="s"><v>1</v></c><c><v>1200.5</v></c><c t="b"><v>1</v></c></row>
            <row><c t="s"><v>99</v></c><c t="s"><v>x</v></c><c t="s"></c></row>
        </sheetData></worksheet>"#;
        let second = r#"<worksheet><sheetData><row><c t="s"><v>2</v></c><c><v>300</v></c></row></sheetData></worksheet>"#;
        let book = zip(&[("xl/sharedStrings.xml", shared), ("xl/worksheets/sheet2.xml", second), ("xl/worksheets/sheet1.xml", first)]);
        assert_eq!(sheet(&book).unwrap(), "Item\tCost\nFlights\t1200.5\t1\n\nHotel\t300");
        // Without shared strings, or with broken ones, the numbers stay.
        let bare = zip(&[("xl/worksheets/sheet1.xml", first)]);
        assert_eq!(sheet(&bare).unwrap(), "\tCost\n\t1200.5\t1");
        let broken = zip(&[("xl/sharedStrings.xml", "<sst"), ("xl/worksheets/sheet1.xml", first)]);
        assert_eq!(sheet(&broken).unwrap(), "\tCost\n\t1200.5\t1");
        assert_eq!(sheet(b"no"), None);
    }

    #[test]
    fn invalid_utf8_still_consumes_the_archive_budget() {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("bad.xml", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(&[0xff, 0xff]).unwrap();
        writer.start_file("good.xml", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(b"<x/>").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let mut zip = open(&bytes).unwrap();
        zip.remaining = 5;
        assert!(part(&mut zip, "bad.xml").is_none());
        assert_eq!(zip.remaining, 3, "unreadable bytes still cost work");
        assert!(part(&mut zip, "good.xml").is_none());
        assert_eq!(zip.remaining, 0);
        assert!(part(&mut zip, "good.xml").is_none());
    }

    #[test]
    fn a_workbook_cannot_expand_shared_strings_without_a_bound() {
        let shared = format!("<sst><si><t>{}</t></si></sst>", "x".repeat(10_000));
        let cells = "<c t=\"s\"><v>0</v></c>".repeat(100);
        let xml = format!("<worksheet><row>{cells}</row></worksheet>");
        let text = sheet(&zip(&[("xl/sharedStrings.xml", &shared), ("xl/worksheets/sheet1.xml", &xml)])).unwrap();
        assert!(text.len() <= crate::MAX_CHARS * 4, "bound output while extracting, before final tidy");
    }

    #[test]
    fn archive_limits_apply_to_the_whole_document() {
        let part = "x".repeat(26 * 1024 * 1024);
        let archive = zip(&[("ppt/slides/slide1.xml", &part), ("ppt/slides/slide2.xml", &part)]);
        assert!(slides(&archive).is_none(), "individually small parts must not evade the aggregate budget");
        let names: Vec<_> = (0..2049).map(|i| format!("part{i}")).collect();
        let entries: Vec<_> = names.iter().map(|name| (name.as_str(), "<x/>")).collect();
        assert!(slides(&zip(&entries)).is_none(), "bound archive-entry work even for ignored parts");
    }

    #[test]
    fn long_documents_stop_at_the_output_budget_without_splitting_unicode() {
        let text = "é".repeat(OUTPUT_BYTES);
        let body = format!("<document><p><t>{text}</t></p></document>");
        let result = word(&zip(&[("word/document.xml", &body), ("word/footnotes.xml", "<p><t>must not fit</t></p>")])).unwrap();
        assert_eq!(result.len(), OUTPUT_BYTES);
        assert!(result.chars().all(|c| c == 'é'));
        let mut bounded = Text("x".repeat(OUTPUT_BYTES - 1));
        bounded.push("é");
        assert_eq!(bounded.0.len(), OUTPUT_BYTES - 1);
        bounded.push("z");
        assert!(bounded.full());
        let too_many_nodes = format!("<root>{}</root>", "<x/>".repeat(MAX_NODES as usize));
        assert!(parse(&too_many_nodes).is_none());
    }

    #[test]
    fn a_part_too_big_to_unpack_is_refused() {
        let huge = format!("<w:document {W}><w:p><w:t>{}</w:t></w:p></w:document>", "a".repeat(PART_BYTES as usize));
        assert!(word(&zip(&[("word/document.xml", &huge)])).is_none());
    }
}
