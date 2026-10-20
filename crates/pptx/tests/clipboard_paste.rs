//! Golden paste-to-PPTX coverage for CLIP-10 / V2-1, with in-memory fixtures.

use std::io::{Cursor, Read};

use deckcraft_engine::Session;
use deckcraft_engine::cmd::{base64_decode, base64_encode};
use quick_xml::Reader;
use quick_xml::events::Event;
use serde_json::json;

fn blank_session() -> Session {
    let mut session = Session::new();
    session.execute("file.new", &json!({"blank": true})).expect("new presentation");
    session.execute("slide.new", &json!({"layout": "blank"})).expect("blank slide");
    session
}

fn save_pptx(session: &mut Session) -> zip::ZipArchive<Cursor<Vec<u8>>> {
    let saved = session.execute("file.saveBytes", &json!({"format": "pptx"})).expect("save PPTX");
    let bytes = base64_decode(saved["data"].as_str().expect("saved data")).expect("PPTX base64");
    zip::ZipArchive::new(Cursor::new(bytes)).expect("PPTX archive")
}

fn part(archive: &mut zip::ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Vec<u8> {
    let mut bytes = vec![];
    archive.by_name(name).unwrap_or_else(|e| panic!("missing part {name}: {e}")).read_to_end(&mut bytes).expect("read part");
    bytes
}

fn xml_part(archive: &mut zip::ZipArchive<Cursor<Vec<u8>>>, name: &str) -> String {
    String::from_utf8(part(archive, name)).expect("UTF-8 XML")
}

fn elements<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let mut reader = Reader::from_str(xml);
    let mut starts = vec![];
    let mut found = vec![];
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event().expect("valid exported XML") {
            Event::Start(e) if e.name().as_ref() == name.as_bytes() => starts.push(start),
            Event::Empty(e) if e.name().as_ref() == name.as_bytes() => {
                found.push(&xml[start..reader.buffer_position() as usize]);
            }
            Event::End(e) if e.name().as_ref() == name.as_bytes() => {
                let start = starts.pop().expect("matching start tag");
                found.push(&xml[start..reader.buffer_position() as usize]);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    assert!(starts.is_empty(), "unclosed {name}");
    found
}

fn only<'a>(xml: &'a str, name: &str) -> &'a str {
    let found = elements(xml, name);
    assert_eq!(found.len(), 1, "expected one {name}: {xml}");
    found[0]
}

fn attr(xml: &str, name: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    match reader.read_event().expect("element XML") {
        Event::Start(e) | Event::Empty(e) => e.attributes().map(|a| a.expect("attribute")).find_map(|a| {
            (a.key.as_ref() == name.as_bytes())
                .then(|| a.decode_and_unescape_value(reader.decoder()).expect("attribute value").into_owned())
        }),
        other => panic!("expected an element, got {other:?}"),
    }
}

fn with_text<'a>(xml: &'a str, name: &str, text: &str) -> &'a str {
    let needle = format!("<a:t>{text}</a:t>");
    let found: Vec<_> = elements(xml, name).into_iter().filter(|e| e.contains(&needle)).collect();
    assert_eq!(found.len(), 1, "expected one {name} containing {needle}");
    found[0]
}

#[test]
fn rich_html_paste_saves_run_and_list_properties() {
    let html = r##"<html><head><style>
.MsoNormal {font-size:16pt;color:#123456}
</style></head><body>
<p class="MsoNormal" style="text-align:center"><b>Word &amp; colour</b></p>
<p style="text-align:right"><span style="font-weight:700;font-size:32px;color:rgb(171,205,239)">Slide text</span></p>
<ul><li>First<ul><li style="text-align:right">Nested</li></ul></li><li>Last</li></ul>
</body></html>"##;
    let mut session = blank_session();
    session.execute("edit.paste", &json!({"html": html, "text": "unused fallback"})).expect("paste HTML");
    let mut archive = save_pptx(&mut session);
    let xml = xml_part(&mut archive, "ppt/slides/slide1.xml");
    let shape = only(&xml, "p:sp");
    assert_eq!(elements(shape, "a:p").len(), 5);
    assert!(!shape.contains("unused fallback"));

    for (text, size, color, align) in [("Word &amp; colour", "1600", "123456", "ctr"), ("Slide text", "2400", "ABCDEF", "r")] {
        let paragraph = with_text(shape, "a:p", text);
        assert_eq!(attr(only(paragraph, "a:pPr"), "algn").as_deref(), Some(align));
        let props = only(with_text(paragraph, "a:r", text), "a:rPr");
        assert_eq!(attr(props, "b").as_deref(), Some("1"));
        assert_eq!(attr(props, "sz").as_deref(), Some(size));
        assert_eq!(attr(only(only(props, "a:solidFill"), "a:srgbClr"), "val").as_deref(), Some(color));
    }
    for (text, level) in [("First", None), ("Nested", Some("1")), ("Last", None)] {
        let props = only(with_text(shape, "a:p", text), "a:pPr");
        assert_eq!(attr(props, "lvl").as_deref(), level);
        assert_eq!(attr(only(props, "a:buChar"), "char").as_deref(), Some("\u{2022}"));
        if text == "Nested" {
            assert_eq!(attr(props, "algn").as_deref(), Some("r"));
        }
    }
}

#[test]
fn merged_html_table_paste_saves_spans_fill_and_text() {
    let html = r##"<table><tr><th colspan="2" rowspan="2" style="background-color:#123456">Merged</th><td>C</td></tr>
<tr><td>F</td></tr><tr><td>G</td><td>H</td><td>I</td></tr></table>"##;
    let mut session = blank_session();
    session.execute("edit.paste", &json!({"html": html, "text": "unused fallback"})).expect("paste merged table");
    let mut archive = save_pptx(&mut session);
    let xml = xml_part(&mut archive, "ppt/slides/slide1.xml");
    let table = only(&xml, "a:tbl");
    assert_eq!(elements(only(table, "a:tblGrid"), "a:gridCol").len(), 3);
    let rows = elements(table, "a:tr");
    assert_eq!(rows.len(), 3);
    let expected = [["Merged", "", "C"], ["", "", "F"], ["G", "H", "I"]];
    for (r, row) in rows.iter().enumerate() {
        let cells = elements(row, "a:tc");
        assert_eq!(cells.len(), 3);
        for (c, cell) in cells.iter().enumerate() {
            assert_eq!(attr(cell, "gridSpan").as_deref(), (r == 0 && c == 0).then_some("2"));
            assert_eq!(attr(cell, "rowSpan").as_deref(), (r == 0 && c == 0).then_some("2"));
            assert_eq!(attr(cell, "hMerge").as_deref(), (r < 2 && c == 1).then_some("1"));
            assert_eq!(attr(cell, "vMerge").as_deref(), (r == 1 && c < 2).then_some("1"));
            if expected[r][c].is_empty() {
                assert!(elements(cell, "a:t").is_empty(), "covered cell {r},{c}");
            } else {
                assert_eq!(only(cell, "a:t"), format!("<a:t>{}</a:t>", expected[r][c]));
            }
        }
    }
    let anchor = elements(rows[0], "a:tc")[0];
    let fill = only(only(anchor, "a:tcPr"), "a:solidFill");
    assert_eq!(attr(only(fill, "a:srgbClr"), "val").as_deref(), Some("123456"));
    assert_eq!(attr(only(with_text(anchor, "a:r", "Merged"), "a:rPr"), "b").as_deref(), Some("1"));
    assert!(!xml.contains("unused fallback"));
}

#[test]
fn tsv_paste_saves_table_grid_and_cell_text() {
    let mut session = blank_session();
    session.execute("edit.paste", &json!({"text": "Name\tValue\r\nAlpha & Beta\t42\r\nGamma\t7\r\n"})).expect("paste TSV");
    let mut archive = save_pptx(&mut session);
    let xml = xml_part(&mut archive, "ppt/slides/slide1.xml");
    let table = only(&xml, "a:tbl");
    assert_eq!(elements(only(table, "a:tblGrid"), "a:gridCol").len(), 2);
    let rows = elements(table, "a:tr");
    assert_eq!(rows.len(), 3);
    let expected = [["Name", "Value"], ["Alpha &amp; Beta", "42"], ["Gamma", "7"]];
    for (row, expected) in rows.iter().zip(expected) {
        let cells = elements(row, "a:tc");
        assert_eq!(cells.len(), 2);
        for (cell, text) in cells.iter().zip(expected) {
            assert_eq!(only(cell, "a:t"), format!("<a:t>{text}</a:t>"));
            for name in ["gridSpan", "rowSpan", "hMerge", "vMerge"] {
                assert_eq!(attr(cell, name), None, "unexpected merge in TSV cell");
            }
        }
    }
}

fn png(rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Cursor::new(vec![]);
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb(rgb)))
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("synthetic PNG");
    out.into_inner()
}

#[test]
fn cross_session_shape_marker_paste_saves_shapes_and_resolvable_media() {
    let source_png = png([20, 100, 220]);
    let destination_png = png([220, 100, 20]);
    let mut source = blank_session();
    let shape = source.execute("shape.insert", &json!({"preset": "ellipse", "text": "Portable shape"})).expect("source shape");
    source.execute("format.bold", &json!({"on": true})).expect("bold source text");
    let picture = source
        .execute("insert.picture", &json!({"name": "source.png", "data": base64_encode(&source_png), "rect": [10, 20, 40, 30]}))
        .expect("source picture");
    source.execute("edit.select", &json!({"ids": [shape["id"], picture["id"]]})).expect("select source shapes");
    let copied = source.execute("edit.copy", &json!({})).expect("copy shapes");
    let html = copied["html"].as_str().expect("HTML clipboard flavor");
    assert!(html.contains("name=\"erpcons-clip\""));

    let mut destination = blank_session();
    destination.execute("shape.insert", &json!({"preset": "rect", "text": "Destination"})).expect("destination shape");
    destination
        .execute("insert.picture", &json!({"name": "destination.png", "data": base64_encode(&destination_png), "rect": [60, 20, 40, 30]}))
        .expect("destination picture");
    assert_eq!(source.active().expect("source").doc.media[0].id, destination.active().expect("destination").doc.media[0].id);
    destination.execute("edit.paste", &json!({"html": html, "text": "unused fallback"})).expect("paste HTML with marker");
    let mut archive = save_pptx(&mut destination);
    let xml = xml_part(&mut archive, "ppt/slides/slide1.xml");
    assert_eq!(elements(&xml, "p:sp").len(), 2);
    assert_eq!(elements(&xml, "p:pic").len(), 2);
    assert_eq!(elements(&xml, "a:blip").len(), 2);
    assert!(!xml.contains("unused fallback"));
    with_text(&xml, "p:sp", "Destination");
    let pasted = with_text(&xml, "p:sp", "Portable shape");
    assert_eq!(attr(only(pasted, "a:prstGeom"), "prst").as_deref(), Some("ellipse"));
    assert_eq!(attr(only(with_text(pasted, "a:r", "Portable shape"), "a:rPr"), "b").as_deref(), Some("1"));

    let rels = xml_part(&mut archive, "ppt/slides/_rels/slide1.xml.rels");
    for picture in elements(&xml, "p:pic") {
        let embed = attr(only(picture, "a:blip"), "r:embed").expect("embedded picture relationship");
        let matches: Vec<_> = elements(&rels, "Relationship").into_iter().filter(|r| attr(r, "Id").as_deref() == Some(embed.as_str())).collect();
        assert_eq!(matches.len(), 1, "unresolved or duplicate r:embed {embed}");
        let relationship = matches[0];
        assert_eq!(attr(relationship, "TargetMode"), None);
        assert_eq!(attr(relationship, "Type").as_deref(), Some("http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"));
        let target = attr(relationship, "Target").expect("media target");
        let file = target.strip_prefix("../media/").expect("slide-relative media target");
        let bytes = part(&mut archive, &format!("ppt/media/{file}"));
        let description = attr(only(picture, "p:cNvPr"), "descr").expect("picture description");
        let expected = match description.as_str() {
            "source.png" => &source_png,
            "destination.png" => &destination_png,
            other => panic!("unexpected picture {other}"),
        };
        assert_eq!(&bytes, expected, "wrong media bytes for {description}");
    }
}
