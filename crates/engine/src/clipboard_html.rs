//! Portable HTML and TSV representations of a copied fragment.

use deckcraft_model::resolve::{self, Ctx};
use deckcraft_model::text::{Anchor, RunKind, RunProps, Strike};
use deckcraft_model::{Fill, MediaItem, Shape, ShapeKind, Table, TextBody};

pub(crate) fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

fn run_style(props: &RunProps, ctx: Option<&Ctx<'_>>) -> String {
    let mut css = String::new();
    if let Some(bold) = props.bold {
        css.push_str(if bold { "font-weight:bold;" } else { "font-weight:normal;" });
    }
    if let Some(italic) = props.italic {
        css.push_str(if italic { "font-style:italic;" } else { "font-style:normal;" });
    }
    let mut decorations = vec![];
    if props.underline.as_ref().is_some_and(|u| u != "none") {
        decorations.push("underline");
    }
    if matches!(props.strike, Some(Strike::Single | Strike::Double)) {
        decorations.push("line-through");
    }
    if !decorations.is_empty() {
        css.push_str(&format!("text-decoration:{};", decorations.join(" ")));
    }
    if let Some(size) = props.size.filter(|s| s.is_finite() && *s > 0.0) {
        css.push_str(&format!("font-size:{size}pt;"));
    }
    if let Some(font) = &props.font {
        let font = ctx.map(|c| c.master.theme.font(font)).unwrap_or_else(|| font.clone());
        // CSS escapes prevent a font name from introducing another declaration.
        let font: String = font
            .chars()
            .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c.to_string() } else { format!("\\{:x} ", c as u32) })
            .collect();
        css.push_str(&format!("font-family:'{font}';"));
    }
    if let Some(Fill::Solid { color }) = &props.fill {
        let color = ctx.map(|c| c.color(color, None)).unwrap_or_else(|| color.resolve(&deckcraft_model::Theme::default().colors, None));
        css.push_str(&format!("color:#{};", color.hex()));
    }
    css
}

pub(crate) fn text(body: &TextBody, ctx: Option<&Ctx<'_>>, shape: Option<&Shape>) -> String {
    let mut html = String::new();
    for para in &body.paragraphs {
        html.push_str("<p style=\"white-space:pre-wrap\">");
        for run in &para.runs {
            let mut props = run.props.clone();
            if let Some(style) = body.list_style.level(para.level) {
                props.inherit(&style.run);
            }
            if let (Some(ctx), Some(shape)) = (ctx, shape) {
                props = resolve::run(ctx, shape, para, &props);
            }
            let content = if matches!(run.kind, RunKind::Break) { "<br>".into() } else { escape(&run.text).replace(['\n', '\u{b}'], "<br>") };
            let css = run_style(&props, ctx);
            if css.is_empty() {
                html.push_str(&content);
            } else {
                html.push_str(&format!("<span style=\"{}\">{content}</span>", escape(&css)));
            }
        }
        html.push_str("</p>");
    }
    html
}

fn tsv_cell(value: &str) -> String {
    if value.contains(['\t', '\n', '\r', '"']) { format!("\"{}\"", value.replace('"', "\"\"")) } else { value.to_owned() }
}

pub(crate) fn table_range(table: &Table, from: (usize, usize), to: (usize, usize)) -> Option<Table> {
    let r0 = from.0.min(to.0);
    let c0 = from.1.min(to.1);
    let r1 = from.0.max(to.0).min(table.n_rows().checked_sub(1)?);
    let c1 = from.1.max(to.1).min(table.n_cols().checked_sub(1)?);
    if r0 > r1 || c0 > c1 {
        return None;
    }
    let mut fragment = table.clone();
    fragment.cols = table.cols.get(c0..=c1)?.to_vec();
    fragment.rows = table
        .rows
        .get(r0..=r1)?
        .iter()
        .map(|row| deckcraft_model::table::Row { height: row.height, cells: (c0..=c1).map(|_| deckcraft_model::Cell::new("")).collect() })
        .collect();
    // A selection may begin inside a merge whose anchor lies outside the selection.
    for (r, row) in table.rows.iter().enumerate().take(r1 + 1) {
        for (c, cell) in row.cells.iter().enumerate().take(c1 + 1) {
            if cell.is_covered() {
                continue;
            }
            let bottom = r.saturating_add(cell.row_span.max(1) as usize - 1).min(r1);
            let right = c.saturating_add(cell.grid_span.max(1) as usize - 1).min(c1);
            let top = r.max(r0);
            let left = c.max(c0);
            if bottom < top || right < left {
                continue;
            }
            for rr in top..=bottom {
                for cc in left..=right {
                    if let Some(dest) = fragment.cell_mut(rr - r0, cc - c0) {
                        if rr == top && cc == left {
                            *dest = cell.clone();
                            dest.grid_span = (right - left + 1) as u32;
                            dest.row_span = (bottom - top + 1) as u32;
                        } else {
                            dest.h_merge = cc > left;
                            dest.v_merge = rr > top;
                        }
                    }
                }
            }
        }
    }
    Some(fragment)
}

pub(crate) fn table(table: &Table, ctx: Option<&Ctx<'_>>) -> (String, String) {
    if table.n_rows() == 0 || table.n_cols() == 0 {
        return (String::new(), String::new());
    }
    let (r1, c1) = (table.n_rows() - 1, table.n_cols() - 1);
    let mut rows = vec![];
    let mut html = String::from("<table style=\"border-collapse:collapse\">");
    for r in 0..=r1 {
        let mut values = vec![];
        html.push_str("<tr>");
        for c in 0..=c1 {
            let cell = table.cell(r, c);
            values.push(tsv_cell(&cell.filter(|x| !x.is_covered()).map(|x| x.text.text()).unwrap_or_default()));
            let Some(cell) = cell else {
                html.push_str("<td></td>");
                continue;
            };
            if cell.is_covered() {
                continue;
            }
            let colspan = (cell.grid_span.max(1) as usize).min(c1 - c + 1);
            let rowspan = (cell.row_span.max(1) as usize).min(r1 - r + 1);
            let mut css = String::new();
            if let Some(Fill::Solid { color }) = &cell.fill {
                let color = ctx.map(|x| x.color(color, None)).unwrap_or_else(|| color.resolve(&deckcraft_model::Theme::default().colors, None));
                css.push_str(&format!("background-color:#{};", color.hex()));
            }
            if let Some(anchor) = cell.anchor {
                let align = match anchor {
                    Anchor::Middle => "middle",
                    Anchor::Bottom => "bottom",
                    _ => "top",
                };
                css.push_str(&format!("vertical-align:{align};"));
            }
            if let Some([left, right, top, bottom]) = cell.margins {
                css.push_str(&format!("padding:{top}pt {right}pt {bottom}pt {left}pt;"));
            }
            for (side, border) in ["left", "right", "top", "bottom"].iter().zip(&cell.borders) {
                if let Some(border) = border
                    && let Some(Fill::Solid { color }) = &border.fill
                {
                    let color = ctx.map(|x| x.color(color, None)).unwrap_or_else(|| color.resolve(&deckcraft_model::Theme::default().colors, None));
                    css.push_str(&format!("border-{side}:{}pt solid #{};", border.width.unwrap_or(0.75), color.hex()));
                }
            }
            html.push_str(&format!("<td colspan=\"{colspan}\" rowspan=\"{rowspan}\" style=\"{}\">", escape(&css)));
            html.push_str(&text(&cell.text, ctx, None));
            html.push_str("</td>");
        }
        html.push_str("</tr>");
        rows.push(values.join("\t"));
    }
    html.push_str("</table>");
    (rows.join("\n"), html)
}

pub(crate) fn shapes(shapes: &[Shape], media: &[MediaItem], ctx: Option<&Ctx<'_>>) -> (String, String) {
    let mut plain = vec![];
    let mut html = String::new();
    deckcraft_model::walk(shapes, &mut |shape, _| {
        if let Some(body) = &shape.text {
            let value = body.text();
            if !value.is_empty() {
                plain.push(value);
            }
            if !body.is_empty() {
                html.push_str(&text(body, ctx, Some(shape)));
            }
        }
        match &shape.kind {
            ShapeKind::Table(t) => {
                let (value, markup) = table(t, ctx);
                if !value.is_empty() {
                    plain.push(value);
                }
                html.push_str(&markup);
            }
            ShapeKind::Picture { fill } => {
                if let Some(item) = media.iter().find(|m| m.id == fill.media)
                    && !item.data.is_empty()
                    && item.data.len() <= 256 * 1024
                    && matches!(item.content_type.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp")
                {
                    html.push_str(&format!("<img src=\"data:{};base64,{}\">", item.content_type, crate::cmd::base64_encode(&item.data)));
                }
            }
            _ => {}
        }
    });
    (plain.join("\n"), html)
}
