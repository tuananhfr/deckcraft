#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Cursor, Write};
use std::sync::Arc;

use super::*;
use crate::DocState;

fn fixture() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let parts = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="bin" ContentType="application/octet-stream"/></Types>"#,
        ),
        ("_rels/.rels", r#"<Relationships><Relationship Id="r1" Type="officeDocument" Target="ppt/presentation.xml"/></Relationships>"#),
        (
            "ppt/presentation.xml",
            r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="s1"/></p:sldIdLst></p:presentation>"#,
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            r#"<Relationships><Relationship Id="s1" Type="slide" Target="slides/slide1.xml"/><Relationship Id="vba" Type="vbaProject" Target="vbaProject.bin"/><Relationship Id="other" Type="customData" Target="customData/item1.xml"/></Relationships>"#,
        ),
        (
            "ppt/slides/slide1.xml",
            r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="2" name="Embedded object"/></p:nvGraphicFramePr><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/presentationml/2006/ole"><p:oleObj r:id="ole1" progId="Synthetic.Object"><p:pic><a:blip r:embed="preview"/></p:pic></p:oleObj></a:graphicData></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>"#,
        ),
        (
            "ppt/slides/_rels/slide1.xml.rels",
            r#"<Relationships><Relationship Id="ole1" Type="oleObject" Target="../embeddings/object1.bin"/><Relationship Id="preview" Type="image" Target="../media/preview.png"/></Relationships>"#,
        ),
        ("ppt/embeddings/object1.bin", "synthetic OLE payload"),
        ("ppt/media/preview.png", "synthetic preview bytes"),
        ("_xmlsignatures/sig1.xml", "<Signature/>"),
        ("ppt/vbaProject.bin", "synthetic macro payload"),
        ("ppt/customData/item1.xml", "<Unsupported/>"),
    ];
    for (name, xml) in parts {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn imported_fixture_reports_ole_signatures_macros_and_dropped_part_kinds() {
    let doc = crate::cmd::file::open_presentation("source.pptm", &fixture()).unwrap();
    let mut session = Session::new();
    session.add_document(DocState::new(doc, None, "source".into())).unwrap();
    assert_eq!(
        session.save_losses("pptx"),
        vec![
            LossItem::OleObjects { count: 1 },
            LossItem::DigitalSignatures,
            LossItem::Macros,
            LossItem::UnsupportedParts { kinds: vec!["customData".into()] },
        ]
    );
    let before = session.active().unwrap().doc.clone();
    let _ = session.save_losses("pdf");
    assert!(Arc::ptr_eq(&before, &session.active().unwrap().doc));
}

#[test]
fn native_roundtrip_keeps_import_loss_provenance() {
    let doc = deckcraft_pptx::import(&fixture()).unwrap();
    let bytes = deckcraft_format::save(&doc).unwrap();
    let loaded = deckcraft_format::load(&bytes).unwrap();
    assert_eq!(losses_for(&loaded, "pptx"), losses_for(&doc, "pptx"));
}

#[test]
fn a_generated_pptx_and_a_new_document_have_no_losses() {
    let doc = Presentation::default();
    assert!(losses_for(&doc, "pptx").is_empty());
    assert!(Session::new().save_losses("pptx").is_empty());
    let doc = deckcraft_pptx::import(&deckcraft_pptx::export(&doc).unwrap()).unwrap();
    assert!(losses_for(&doc, "pptx").is_empty(), "{:?}", doc.import_losses);
}

#[test]
fn nested_ole_counts_change_when_objects_are_removed() {
    let mut doc = deckcraft_pptx::import(&fixture()).unwrap();
    let slide = Arc::make_mut(doc.slides.first_mut().unwrap());
    let object = slide.shapes.first().unwrap().clone();
    slide.shapes = vec![deckcraft_model::Shape {
        kind: ShapeKind::Group { children: vec![object.clone(), object], child: Default::default() },
        ..Default::default()
    }];
    assert!(losses_for(&doc, "pptx").contains(&LossItem::OleObjects { count: 2 }));
    Arc::make_mut(doc.slides.first_mut().unwrap()).shapes.clear();
    assert!(!losses_for(&doc, "pptx").iter().any(|loss| matches!(loss, LossItem::OleObjects { .. })));
}

#[test]
fn relationship_free_opaque_xml_is_preserved_without_a_warning() {
    let mut doc = Presentation::default();
    Arc::make_mut(doc.slides.first_mut().unwrap()).shapes.push(deckcraft_model::Shape {
        kind: ShapeKind::Opaque {
            xml: "<p:graphicFrame xmlns:p=\"urn:synthetic\"><custom/></p:graphicFrame>".into(),
            preview: None,
            label: "Custom".into(),
        },
        ..Default::default()
    });
    assert!(losses_for(&doc, "pptx").is_empty());
}
