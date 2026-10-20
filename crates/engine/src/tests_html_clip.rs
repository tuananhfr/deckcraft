#![allow(clippy::unwrap_used, clippy::expect_used)]

use deckcraft_model::text::{Action, Bullet, RunKind, Strike};
use deckcraft_model::{ColorRef, Fill};

use crate::{html_clip, html_tokens, table_paste};

fn rgb(r: u8, g: u8, b: u8) -> ColorRef {
    ColorRef::rgb(deckcraft_color::Rgba::rgb(r, g, b))
}

#[test]
fn office_html_samples_keep_run_styles() {
    let samples = [
        r##"<html><head><style>p {font-size:16pt} .MsoNormal {color:#123456;font-family:'Inter',sans-serif}</style></head><body><p class="MsoNormal"><b>Word</b></p></body></html>"##,
        r##"<div><span style="font-family:Inter;font-size:16pt;color:rgb(18,52,86);font-weight:700">PowerPoint</span></div>"##,
        r##"<b id="docs-internal-guid"><p><span style="font-family:Inter;font-size:21.333333333333332px;color:#123456;font-weight:600">Slides</span></p></b>"##,
        r##"<style>.xl65 {font-family:Inter;font-size:16pt;color:#123456;font-weight:bold}</style><table><tr><td class="xl65">Excel</td></tr></table>"##,
    ];
    for html in samples {
        let clip = html_clip::read(html).unwrap();
        let run = &clip.text.paragraphs[0].runs[0];
        assert_eq!(run.props.bold, Some(true));
        assert_eq!(run.props.fill, Some(Fill::solid(rgb(18, 52, 86))));
        assert!((run.props.size.unwrap() - 16.0).abs() < 1e-9);
        assert_eq!(run.props.font.as_deref(), Some("Inter"));
    }
}

#[test]
fn inline_styles_headings_breaks_and_entities() {
    let clip = html_clip::read(r##"<h2>Heading</h2><div><strong>A</strong> <em>B</em><br><u>C</u><strike>D</strike><s>E</s><span style="background-color:#abcdef;text-decoration:underline line-through;font-style:italic">&#x1F642;&nbsp;&amp;</span></div>"##).unwrap();
    assert_eq!(clip.text.paragraphs.len(), 2);
    assert_eq!(clip.text.paragraphs[0].runs[0].props.size, Some(24.0));
    let runs = &clip.text.paragraphs[1].runs;
    assert!(runs.iter().any(|r| r.kind == RunKind::Break));
    assert!(runs.iter().any(|r| r.props.italic == Some(true)));
    let last = runs.last().unwrap();
    assert_eq!(last.text, "\u{1f642}\u{a0}&");
    assert_eq!(last.props.highlight, Some(rgb(171, 205, 239)));
    assert_eq!(last.props.strike, Some(Strike::Single));
    assert_eq!(last.props.underline.as_deref(), Some("sng"));
}

#[test]
fn css_class_specificity_inline_override_and_late_styles() {
    let clip = html_clip::read(r##"<p class="picked"><span style="font-size:19pt">X</span><span>Y</span></p><style>.picked {color:#123456;font-size:14pt} p {color:red;font-size:12pt} p span {font-weight:bold} #picked {font-style:italic}</style>"##).unwrap();
    let runs = &clip.text.paragraphs[0].runs;
    assert_eq!(runs[0].props.size, Some(19.0));
    assert_eq!(runs[1].props.size, Some(14.0));
    assert_eq!(runs[0].props.fill, Some(Fill::solid(rgb(18, 52, 86))));
    assert_eq!(runs[0].props.bold, None);
    assert_eq!(runs[0].props.italic, None);
}

#[test]
fn lists_keep_nested_levels_and_numbering() {
    let clip = html_clip::read("<ul><li>One<ol start='3'><li>Nested</li><li>Next</li></ol></li><li>Two</li></ul>").unwrap();
    let p = &clip.text.paragraphs;
    assert_eq!(
        p.iter().map(|p| (p.text(), p.level)).collect::<Vec<_>>(),
        vec![("One".into(), 0), ("Nested".into(), 1), ("Next".into(), 1), ("Two".into(), 0)]
    );
    assert!(matches!(&p[0].props.bullet, Some(Bullet::Char { .. })));
    assert!(matches!(&p[1].props.bullet, Some(Bullet::AutoNum { start_at: 3, .. })));
    assert!(matches!(&p[2].props.bullet, Some(Bullet::AutoNum { start_at: 4, .. })));
}

pub(crate) const MERGED_TABLE: &str = r##"<table><tr><th colspan="2" rowspan="2" style="background-color:#123456">A</th><td>C</td></tr><tr><td>F</td></tr><tr><td>G</td><td>H</td><td>I</td></tr></table>"##;

#[test]
fn html_table_three_by_three_keeps_merge_fill_and_bold() {
    let table = html_clip::read(MERGED_TABLE).unwrap().table.unwrap();
    assert_eq!((table.n_rows(), table.n_cols()), (3, 3));
    let anchor = table.cell(0, 0).unwrap();
    assert_eq!((anchor.grid_span, anchor.row_span), (2, 2));
    assert_eq!(anchor.text.text(), "A");
    assert_eq!(anchor.text.paragraphs[0].runs[0].props.bold, Some(true));
    assert_eq!(anchor.fill, Some(Fill::solid(rgb(18, 52, 86))));
    assert!(table.cell(1, 1).unwrap().is_covered());
    assert_eq!(table.cell(1, 2).unwrap().text.text(), "F");
    assert_eq!(table.cell(2, 2).unwrap().text.text(), "I");
}

#[test]
fn table_text_retains_formatting_for_shape_editing() {
    let clip = html_clip::read("<p>Before</p><table><tr><td><b>A</b></td><td>B</td></tr></table><p>After</p>").unwrap();
    assert_eq!(clip.text.text(), "Before\nA\nB\nAfter");
    assert_eq!(clip.text.paragraphs[1].runs[0].props.bold, Some(true));
}

#[test]
fn unsafe_content_is_dropped_and_unknown_tag_text_survives() {
    let clip = html_clip::read(r##"<script>bad()</script><script>x</scriptx>bad</script><style>p {background-image:url(https://bad)}</style><p onclick="bad()"><img src="https://bad" onerror="bad()"><a href="javascript:bad()">J</a><a href="data:text/html,bad">D</a><custom>Safe</custom><span style="font-size:NaNpt;position:fixed">Text</span></p>"##).unwrap();
    assert_eq!(clip.text.text(), "JDSafeText");
    for run in &clip.text.paragraphs[0].runs {
        assert!(run.props.link.is_none());
        assert!(run.props.size.is_none());
    }
}

#[test]
fn only_http_https_mailto_links_are_kept() {
    for href in ["http://example.test", "HTTPS://example.test", "mailto:a@example.test"] {
        let clip = html_clip::read(&format!("<a href='{href}'>Link</a>")).unwrap();
        assert!(matches!(&clip.text.paragraphs[0].runs[0].props.link.as_ref().unwrap().action, Action::Url { url } if url == href));
    }
    for href in ["javascript&#58;bad()", "data:text/plain,bad", "file:///bad", "https&#10;:bad", "/relative"] {
        let clip = html_clip::read(&format!("<a href='{href}'>Link</a>")).unwrap();
        assert!(clip.text.paragraphs[0].runs[0].props.link.is_none());
    }
}

#[test]
fn hostile_limits_reject_without_recursive_parsing_or_grid_expansion() {
    let deep = format!("{}X{}", "<span>".repeat(65), "</span>".repeat(65));
    let cells = format!("<table><tr>{}</tr></table>", "<td>x</td>".repeat(100_000));
    let huge_style = format!("<p style='{}'>X</p>", "x".repeat(html_tokens::MAX_STYLE + 1));
    let huge_css = format!("<style>{}</style><p>X</p>", "x".repeat(html_tokens::MAX_STYLE + 1));
    for html in [
        deep,
        cells,
        huge_style,
        huge_css,
        "x".repeat(html_tokens::MAX_INPUT + 1),
        "<table><tr><td colspan='20000' rowspan='20000'>X</td></tr></table>".into(),
    ] {
        assert!(html_clip::read(&html).is_none());
    }
    assert!(html_clip::read(&format!("{}X{}", "<span>".repeat(64), "</span>".repeat(64))).is_some());
}

#[test]
fn malformed_and_multibyte_fragments_never_require_valid_xml() {
    for html in [
        "<div><b>X</div>",
        "Text < 4",
        "<p title='a > b'>X",
        "<p style='font-size:Infinitypt'>X",
        "<span class='\u{1f642}'>Text</span>",
        "<p = / a='x'>Text</p>",
        "<p><a href=javascript:bad()>Text</a>",
    ] {
        assert!(html_clip::read(html).is_some(), "{html}");
    }
    assert_eq!(html_clip::plain("<script>bad</script><p>Kept</p>"), "Kept");
    assert_eq!(html_clip::plain(&format!("<p style='{}'>Kept</p>", "x".repeat(8193))), "Kept");
}

#[test]
fn tsv_three_by_two_and_quoted_cells() {
    let table = table_paste::tsv("A\tB\r\nC\tD\r\nE\tF\r\n").unwrap();
    assert_eq!((table.n_rows(), table.n_cols()), (3, 2));
    assert_eq!(table.cell(2, 1).unwrap().text.text(), "F");
    let table = table_paste::tsv("\"a\tb\"\t\"two\nlines\"\n\"say \"\"hi\"\"\"\tz").unwrap();
    assert_eq!(table.cell(0, 0).unwrap().text.text(), "a\tb");
    assert_eq!(table.cell(0, 1).unwrap().text.text(), "two\nlines");
    assert_eq!(table.cell(1, 0).unwrap().text.text(), "say \"hi\"");
}

#[test]
fn non_grid_and_oversized_tsv_stay_plain() {
    for text in ["one\ttwo", "one\ntwo", "one\ttwo\nthree", "\"open\tquote\nnext\tfield"] {
        assert!(table_paste::tsv(text).is_none());
    }
    assert!(table_paste::tsv(&"A\tB\n".repeat(10_001)).is_none());
}
