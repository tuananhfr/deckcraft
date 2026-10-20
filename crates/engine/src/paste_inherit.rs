//! Freeze placeholder text inheritance before a paste replaces the placeholder.

use deckcraft_model::resolve;
use deckcraft_model::text::{Paragraph, RunProps, TextBody};
use deckcraft_model::{ColorRef, Fill};

use crate::{Result, Session, Target};

pub(crate) fn prepare(s: &mut Session, source: &TextBody) -> Result<TextBody> {
    let st = s.doc()?;
    let Some(t) = st.selection.text.as_ref().filter(|t| !t.notes && t.cell.is_none()) else { return Ok(source.clone()) };
    let Some(shape) = st.shape(t.shape).filter(|shape| shape.ph.is_some()) else { return Ok(source.clone()) };
    let Some(ctx) = crate::cmd::text::ctx_for(&st.doc, &st.selection) else { return Ok(source.clone()) };
    let original = shape.text.clone().unwrap_or_default();
    let caret_para = original.paragraphs.get(t.ordered().0.0).cloned().unwrap_or_default();
    let mut incoming = source.clone();
    let explicit = deckcraft_model::edit::props_at(&original, t.ordered().0);
    for p in &mut incoming.paragraphs {
        let para = Paragraph { level: if p.props.bullet.is_some() { p.level } else { caret_para.level }, ..caret_para.clone() };
        let props = concrete(&ctx, shape, &para, &explicit);
        p.props.inherit(&resolve::para(&ctx, shape, &para));
        p.end_props.inherit(&props);
        for run in &mut p.runs {
            run.props.inherit(&props);
        }
    }
    let (a, b) = t.ordered();
    let replaces = original.is_empty() || (a == (0, 0) && b == deckcraft_model::edit::end(&original));
    if !replaces || st.selection.target != Target::Slides {
        return Ok(incoming);
    }
    let mut frozen = shape.clone();
    frozen.xfrm = Some(crate::cmd::xfrm_of(&st.doc, &st.selection, shape));
    let mut body = original;
    body.body = resolve::body(&ctx, shape);
    for p in &mut body.paragraphs {
        p.props = resolve::para(&ctx, shape, p);
        p.end_props = concrete(&ctx, shape, p, &p.end_props);
        let para = p.clone();
        for run in &mut p.runs {
            run.props = concrete(&ctx, shape, &para, &run.props);
        }
    }
    // The first paragraph also needs defaults when the inherited placeholder has no text yet.
    if body.paragraphs.is_empty() {
        body.paragraphs.push(Paragraph {
            props: resolve::para(&ctx, shape, &caret_para),
            end_props: concrete(&ctx, shape, &caret_para, &explicit),
            ..Default::default()
        });
    }
    frozen.text = Some(body);
    frozen.ph = None;
    let id = frozen.id;
    s.edit(|doc, sel| {
        if let Some(shapes) = crate::shapes_mut(doc, sel)
            && let Some(shape) = deckcraft_model::find_shape_mut(shapes, id)
        {
            *shape = frozen;
        }
        Ok(())
    })?;
    Ok(incoming)
}

fn concrete(ctx: &resolve::Ctx<'_>, shape: &deckcraft_model::Shape, para: &Paragraph, props: &RunProps) -> RunProps {
    let mut props = resolve::run(ctx, shape, para, props);
    props.font = Some(resolve::font_family(ctx, &props));
    if props.fill.is_none() {
        props.fill = Some(Fill::solid(ColorRef::rgb(resolve::text_color(ctx, &props))));
    }
    props
}
