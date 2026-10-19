//! Decompression limits: zip bombs, too many entries and bombs nested in embedded packages are
//! refused as a whole; a large legitimate deck still opens.

use std::io::{Cursor, Write};

use deckcraft_model::inflate::{LIMITS, TOO_LARGE};
use deckcraft_pptx::PptxError;
use serde_json::json;

fn zip_with(files: &[(&str, &[u8])], o: zip::write::SimpleFileOptions) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (n, d) in files {
        z.start_file(*n, o).expect("start");
        z.write_all(d).expect("write");
    }
    z.finish().expect("finish").into_inner()
}

fn stored() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)
}

/// Every entry of a valid export, with `extra` added or replacing the entry of that name, deflated.
fn with_parts(extra: &[(&str, &[u8])]) -> Vec<u8> {
    let valid = deckcraft_pptx::export(&deckcraft_model::defaults::new_presentation(None)).expect("export");
    let mut z = zip::ZipArchive::new(Cursor::new(valid)).expect("zip");
    let mut es: Vec<(String, Vec<u8>)> = (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).expect("entry");
            let mut d = vec![];
            std::io::Read::read_to_end(&mut f, &mut d).expect("read");
            (f.name().to_string(), d)
        })
        .collect();
    es.retain(|(n, _)| !extra.iter().any(|(e, _)| e == n));
    es.extend(extra.iter().map(|(n, d)| (n.to_string(), d.to_vec())));
    let v: Vec<(&str, &[u8])> = es.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    zip_with(&v, zip::write::SimpleFileOptions::default())
}

fn refused(r: Result<deckcraft_model::Presentation, PptxError>) -> String {
    match r {
        Err(e @ PptxError::TooLarge(_)) => {
            let m = e.to_string();
            assert!(m.starts_with(TOO_LARGE), "{m}");
            m
        }
        Err(e) => panic!("refused for another reason: {e}"),
        Ok(_) => panic!("the hostile package opened"),
    }
}

#[test]
fn a_deflate_bomb_is_refused_by_its_ratio() {
    let bytes = with_parts(&[("ppt/slides/slide1.xml", &vec![b' '; 64 << 20])]);
    assert!(bytes.len() < 1 << 20);
    assert!(refused(deckcraft_pptx::import(&bytes)).contains("compressed more than 200:1"));
}

#[test]
fn too_many_entries_are_refused() {
    let names: Vec<String> = (0..=LIMITS.entries).map(|i| format!("customXml/i{i}.xml")).collect();
    let mut parts: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b""[..])).collect();
    parts.push(("[Content_Types].xml", b"<Types/>"));
    refused(deckcraft_pptx::import(&zip_with(&parts, stored())));
}

#[test]
fn a_bomb_nested_in_an_embedded_package_is_checked_where_it_inflates() {
    // Zip-in-zip with the inner bomb stored: the outer entry is what inflates, and is refused.
    let inner = zip_with(&[("xl/worksheets/sheet1.xml", &vec![b' '; 32 << 20])], stored());
    refused(deckcraft_pptx::import(&with_parts(&[("ppt/embeddings/Microsoft_Excel_Worksheet.xlsx", &inner)])));
    // The inner bomb deflated: kept as opaque bytes, never inflated, so the deck opens.
    let inner = zip_with(&[("xl/worksheets/sheet1.xml", &vec![b' '; 64 << 20])], zip::write::SimpleFileOptions::default());
    let p = deckcraft_pptx::import(&with_parts(&[("ppt/embeddings/Microsoft_Excel_Worksheet.xlsx", &inner)])).expect("import");
    assert_eq!(p.slides.len(), 1);
}

/// Rewrites the inflated size `name` declares, in its local header and the central directory.
fn declare_size(zip: &mut [u8], name: &str, size: u32) {
    for i in 0..zip.len().saturating_sub(4) {
        let (name_at, size_at) = match zip[i..i + 4] {
            [0x50, 0x4b, 0x03, 0x04] => (30, 22),
            [0x50, 0x4b, 0x01, 0x02] => (46, 24),
            _ => continue,
        };
        if zip.get(i + name_at..i + name_at + name.len()) == Some(name.as_bytes()) {
            zip[i + size_at..i + size_at + 4].copy_from_slice(&size.to_le_bytes());
        }
    }
}

#[test]
fn a_required_part_declared_over_the_cap_is_refused_as_too_large() {
    // Honest headers on a part too large for this build: not inflated, and not reported as a
    // file missing its presentation part either.
    let over = u32::try_from(LIMITS.entry + 1).expect("fits the zip32 field");
    for part in ["ppt/presentation.xml", "[Content_Types].xml"] {
        let mut bytes = with_parts(&[]);
        declare_size(&mut bytes, part, over);
        assert!(refused(deckcraft_pptx::import(&bytes)).contains("declares"), "{part}");
    }
}

#[test]
fn a_large_legitimate_deck_still_opens() {
    let mut outline = String::new();
    for i in 0..100 {
        outline.push_str(&format!("Báo cáo tiến độ công trình {i}\n"));
        for j in 0..8 {
            outline.push_str(&format!("\tHạng mục {j}: hoàn thành {}% khối lượng, chi phí {} triệu đồng\n", (i * 7 + j) % 100, i * 13 + j));
        }
    }
    let mut s = deckcraft_engine::Session::new();
    s.execute("file.new", &json!({})).expect("new");
    s.execute("slide.fromOutline", &json!({"text": outline})).expect("outline");
    let deck = s.doc().expect("doc").doc.clone();
    assert!(deck.slides.len() >= 100);
    let bytes = deckcraft_pptx::export(&deck).expect("export");
    let back = deckcraft_pptx::import(&bytes).expect("import");
    assert_eq!(back.slides.len(), deck.slides.len());
}
