//! Bounded clipboard grids and atomic table insertion/replacement.

use std::collections::BTreeMap;

use deckcraft_geom::Xfrm;
use deckcraft_model::table::Row;
use deckcraft_model::{Cell, Shape, ShapeId, ShapeKind, Table};
use serde_json::{Value, json};

use crate::{Result, Session, html_tokens::MAX_INPUT};

pub(crate) const MAX_CELLS: usize = 20_000;

pub(crate) fn from_rows(rows: Vec<Vec<Cell>>) -> Option<Table> {
    let mut grid = BTreeMap::new();
    let mut nr = rows.len();
    let mut nc = 0;
    if nr == 0 || nr > MAX_CELLS {
        return None;
    }
    for (r, row) in rows.into_iter().enumerate() {
        let mut c = 0usize;
        for cell in row {
            let rs = cell.row_span.max(1) as usize;
            let cs = cell.grid_span.max(1) as usize;
            while grid.contains_key(&(r, c)) {
                c = c.checked_add(1)?;
            }
            let bottom = r.checked_add(rs)?;
            let right = c.checked_add(cs)?;
            nr = nr.max(bottom);
            nc = nc.max(right);
            if nr.checked_mul(nc)? > MAX_CELLS {
                return None;
            }
            for rr in r..bottom {
                for cc in c..right {
                    if grid.contains_key(&(rr, cc)) {
                        return None;
                    }
                    let mut covered = Cell::new("");
                    covered.h_merge = cc > c;
                    covered.v_merge = rr > r;
                    grid.insert((rr, cc), covered);
                }
            }
            grid.insert((r, c), cell);
            c = right;
        }
    }
    if nc == 0 {
        return None;
    }
    let rows =
        (0..nr).map(|r| Row { height: 30.0, cells: (0..nc).map(|c| grid.remove(&(r, c)).unwrap_or_else(|| Cell::new(""))).collect() }).collect();
    Some(Table { cols: vec![80.0; nc], rows, style: "none".into(), first_row: false, band_row: false, ..Default::default() })
}

pub(crate) fn tsv(text: &str) -> Option<Table> {
    if text.len() > MAX_INPUT || !text.contains('\t') {
        return None;
    }
    let mut rows = vec![];
    let mut row = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut count = 0;
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(ch);
            }
            continue;
        }
        match ch {
            '"' if field.is_empty() => quoted = true,
            '\t' | '\r' | '\n' => {
                count += 1;
                if count > MAX_CELLS {
                    return None;
                }
                row.push(Cell::new(&field));
                field.clear();
                if ch != '\t' {
                    if ch == '\r' && chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    rows.push(std::mem::take(&mut row));
                }
            }
            _ => field.push(ch),
        }
    }
    if quoted {
        return None;
    }
    if !field.is_empty() || !row.is_empty() {
        count += 1;
        if count > MAX_CELLS {
            return None;
        }
        row.push(Cell::new(&field));
        rows.push(row);
    }
    if rows.iter().filter(|r| r.len() >= 2).count() < 2 {
        return None;
    }
    from_rows(rows)
}

fn target(s: &Session) -> Option<(ShapeId, usize, usize)> {
    let st = s.active()?;
    let sel = &st.selection;
    let (id, r, c) = if let Some(text) = &sel.text {
        if text.notes {
            return None;
        }
        let (r, c) = text.cell?;
        (text.shape, r, c)
    } else if let Some((id, a, b)) = sel.cells {
        (id, a.0.min(b.0), a.1.min(b.1))
    } else {
        (*sel.shapes.first()?, 0, 0)
    };
    let ShapeKind::Table(table) = &st.shape(id)?.kind else { return None };
    let cell = table.cell(r, c)?;
    if cell.is_covered() {
        for (rr, row) in table.rows.iter().enumerate().take(r + 1) {
            for (cc, cell) in row.cells.iter().enumerate().take(c + 1) {
                if !cell.is_covered() && rr.saturating_add(cell.row_span.max(1) as usize) > r && cc.saturating_add(cell.grid_span.max(1) as usize) > c
                {
                    return Some((id, rr, cc));
                }
            }
        }
        return None;
    }
    Some((id, r, c))
}

pub(crate) fn has_target(s: &Session) -> bool {
    target(s).is_some()
}

pub(crate) fn paste(s: &mut Session, mut source: Table) -> Result<Value> {
    if let Some((id, r, c)) = target(s) {
        let fits = s.doc()?.shape(id).is_some_and(|shape| {
            let ShapeKind::Table(table) = &shape.kind else { return false };
            r.checked_add(source.n_rows())
                .zip(c.checked_add(source.n_cols()))
                .and_then(|(nr, nc)| nr.max(table.n_rows()).checked_mul(nc.max(table.n_cols())))
                .is_some_and(|n| n <= MAX_CELLS)
        });
        if !fits {
            let (plain, _) = crate::clipboard_html::table(&source, None);
            return crate::rich_paste::plain(s, &plain);
        }
        return s.edit(|doc, sel| {
            let shapes = crate::shapes_mut(doc, sel).ok_or(crate::EngineError::NoDocument)?;
            let shape = deckcraft_model::find_shape_mut(shapes, id).ok_or(crate::EngineError::NoDocument)?;
            let ShapeKind::Table(table) = &mut shape.kind else {
                return Ok(json!({}));
            };
            let (bottom, right) = (r + source.n_rows(), c + source.n_cols());
            while table.n_cols() < right {
                table.insert_col(table.n_cols());
            }
            while table.n_rows() < bottom {
                table.insert_row(table.n_rows());
            }
            table.normalize();
            split_intersections(table, r, c, bottom, right);
            for (rr, row) in source.rows.iter().enumerate() {
                for (cc, cell) in row.cells.iter().enumerate() {
                    if let Some(dest) = table.cell_mut(r + rr, c + cc) {
                        *dest = cell.clone();
                    }
                }
            }
            if let Some(x) = shape.xfrm.as_mut() {
                x.w = table.width();
                x.h = table.height();
            }
            sel.shapes = vec![id];
            sel.text = None;
            sel.cells = Some((id, (r, c), (bottom - 1, right - 1)));
            Ok(json!({"id": id}))
        });
    }
    let size = s.doc()?.doc.slide_size;
    let w = size.width * 0.8;
    let h = (30.0 * source.n_rows() as f64).min(size.height * 0.8);
    let cw = w / source.n_cols().max(1) as f64;
    let rh = h / source.n_rows().max(1) as f64;
    source.cols.fill(cw);
    for row in &mut source.rows {
        row.height = rh;
    }
    let shape =
        Shape { xfrm: Some(Xfrm::new(size.width * 0.1, (size.height - h) / 2.0, w, h)), kind: ShapeKind::Table(source), ..Default::default() };
    let id = crate::cmd::insert::add_shape(s, shape, true)?;
    s.select(|_, sel| sel.cells = None)?;
    Ok(json!({"id": id}))
}

fn split_intersections(table: &mut Table, r: usize, c: usize, bottom: usize, right: usize) {
    let mut spans = vec![];
    for (rr, row) in table.rows.iter().enumerate() {
        for (cc, cell) in row.cells.iter().enumerate() {
            let rb = rr.saturating_add(cell.row_span.max(1) as usize).min(table.n_rows());
            let cr = cc.saturating_add(cell.grid_span.max(1) as usize).min(table.n_cols());
            if !cell.is_covered() && rr < bottom && cc < right && rb > r && cr > c {
                spans.push((rr, cc, rb, cr));
            }
        }
    }
    // Split the entire old merge, including covered cells outside the pasted rectangle.
    for (rr, cc, rb, cr) in spans {
        for r in rr..rb {
            for c in cc..cr {
                if let Some(cell) = table.cell_mut(r, c) {
                    cell.grid_span = 1;
                    cell.row_span = 1;
                    cell.h_merge = false;
                    cell.v_merge = false;
                }
            }
        }
    }
}
