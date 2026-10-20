//! Slide content bounds and renderer-aware sizing for newly pasted tables.

use deckcraft_geom::Xfrm;
use deckcraft_model::resolve::{self, Ctx};
use deckcraft_model::{PhType, Table};

use crate::{Result, Session};

const DEFAULT_FONT_SIZE: f64 = 18.0;
const MIN_FONT_SIZE: f64 = 8.0;
const DEFAULT_ROW_HEIGHT: f64 = 30.0;

fn content_area(s: &Session) -> Result<Xfrm> {
    let state = s.doc()?;
    let size = state.doc.slide_size;
    let margins = Xfrm::new(size.width * 0.1, size.height * 0.1, size.width * 0.8, size.height * 0.8);
    let body = state.current_slide().and_then(|slide| {
        let (master, Some(layout)) = state.doc.master_for(slide)? else { return None };
        let ctx = Ctx::for_layout(&state.doc, master, layout);
        layout
            .shapes
            .iter()
            .find(|shape| matches!(shape.ph_type(), Some(PhType::Body | PhType::Obj | PhType::Table)))
            .map(|shape| resolve::xfrm(&ctx, shape))
    });
    if let Some(body) = body.filter(|b| [b.x, b.y, b.w, b.h].iter().all(|v| v.is_finite()) && b.w > 0.0 && b.h > 0.0) {
        let x = body.x.max(margins.x);
        let y = body.y.max(margins.y);
        let right = (body.x + body.w).min(margins.x + margins.w);
        let bottom = (body.y + body.h).min(margins.y + margins.h);
        if right > x && bottom > y {
            return Ok(Xfrm::new(x, y, right - x, bottom - y));
        }
    }
    Ok(margins)
}

fn scaled(source: &Table, scale: f64) -> Table {
    let mut table = source.clone();
    for row in &mut table.rows {
        row.height = (DEFAULT_ROW_HEIGHT * scale).max(MIN_FONT_SIZE);
        for cell in &mut row.cells {
            if scale < 1.0 {
                // Fixed padding would force rendered rows to overflow even at the font-size floor.
                let mut margins = cell.margins.unwrap_or([7.2, 7.2, 3.6, 3.6]);
                for margin in &mut margins {
                    *margin *= scale;
                }
                cell.margins = Some(margins);
            }
            for para in &mut cell.text.paragraphs {
                for props in std::iter::once(&mut para.end_props).chain(para.runs.iter_mut().map(|run| &mut run.props)) {
                    let size = props.size.filter(|size| size.is_finite() && *size > 0.0).unwrap_or(DEFAULT_FONT_SIZE);
                    props.size = Some((size * scale).max(MIN_FONT_SIZE));
                }
            }
        }
    }
    table
}

fn measure(s: &Session, mut table: Table) -> Result<Table> {
    let state = s.doc()?;
    // Stored heights are minima; wrapped text and cell padding can make the rendered rows taller.
    let heights = crate::cmd::table_drawn_heights(&state.doc, &state.selection, &table);
    for (row, height) in table.rows.iter_mut().zip(heights) {
        row.height = height;
    }
    Ok(table)
}

pub(super) fn fit(s: &Session, source: &mut Table) -> Result<Xfrm> {
    let area = content_area(s)?;
    let col_width = area.w / source.n_cols().max(1) as f64;
    source.cols.fill(col_width);
    let mut fitted = measure(s, scaled(source, 1.0))?;
    // Leave room for floating-point accumulation when the last row meets the content edge.
    let available = (area.h - 1e-6).max(0.0);
    if fitted.height() > available {
        fitted = measure(s, scaled(source, 0.0))?;
        if fitted.height() <= available {
            let (mut low, mut high) = (0.0, 1.0);
            for _ in 0..8 {
                let scale = (low + high) / 2.0;
                let candidate = measure(s, scaled(source, scale))?;
                if candidate.height() <= available {
                    low = scale;
                    fitted = candidate;
                } else {
                    high = scale;
                }
            }
        }
    }
    let rect = Xfrm::new(area.x, area.y, fitted.width(), fitted.height());
    *source = fitted;
    Ok(rect)
}
