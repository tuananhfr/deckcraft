//! Allow-listed clipboard HTML translated directly into engine text and table models.

use deckcraft_model::text::{Action, Bullet, Hyperlink, Paragraph, Run, RunKind, TextBody};
use deckcraft_model::{Cell, Fill, Table};

use crate::html_style::{Rules, Style};
use crate::html_tokens::{MAX_INPUT, MAX_STYLE, Token, Tokens, decode};
use crate::table_paste::{self, MAX_CELLS};

pub(crate) struct Clip {
    pub text: TextBody,
    pub table: Option<Table>,
}

struct Frame {
    name: String,
    style: Style,
    list: Option<Bullet>,
}

#[derive(Default)]
struct Body {
    paragraphs: Vec<Paragraph>,
    current: Option<Paragraph>,
    space: bool,
}

impl Body {
    fn flush(&mut self, keep_empty: bool) {
        if let Some(mut p) = self.current.take() {
            for run in p.runs.iter_mut().rev() {
                run.text.truncate(run.text.trim_end_matches(' ').len());
                if !run.text.is_empty() || run.kind != RunKind::Text {
                    break;
                }
            }
            p.normalize();
            if keep_empty || !p.is_empty() {
                self.paragraphs.push(p);
            }
        }
        self.space = false;
    }

    fn start(&mut self, style: &Style, level: u8) {
        self.flush(false);
        self.current = Some(Paragraph { level, props: style.para.clone(), end_props: style.run.clone(), ..Default::default() });
    }

    fn text(&mut self, raw: &str, style: &Style, level: u8) {
        let mut text = String::new();
        let has_text = self.current.as_ref().is_some_and(|p| !p.runs.is_empty());
        for ch in decode(raw).chars() {
            if matches!(ch, ' ' | '\t' | '\r' | '\n' | '\u{c}') {
                if has_text || !text.is_empty() {
                    self.space = true;
                }
            } else {
                if self.space {
                    text.push(' ');
                }
                self.space = false;
                text.push(ch);
            }
        }
        if text.is_empty() {
            return;
        }
        let p =
            self.current.get_or_insert_with(|| Paragraph { level, props: style.para.clone(), end_props: style.run.clone(), ..Default::default() });
        if let Some(last) = p.runs.last_mut()
            && last.kind == RunKind::Text
            && last.props == style.run
        {
            last.text.push_str(&text);
        } else {
            p.runs.push(Run::with(text, style.run.clone()));
        }
    }

    fn line_break(&mut self, style: &Style, level: u8) {
        if self.current.is_none() {
            self.start(style, level);
        }
        if let Some(p) = &mut self.current {
            p.runs.push(Run { text: String::new(), props: style.run.clone(), kind: RunKind::Break });
        }
        self.space = false;
    }

    fn finish(mut self) -> TextBody {
        self.flush(false);
        TextBody { paragraphs: self.paragraphs, ..Default::default() }
    }
}

#[derive(Default)]
struct Grid {
    rows: Vec<Vec<Cell>>,
    row: Vec<Cell>,
    cell: Option<(Cell, Body)>,
    count: usize,
    text: Vec<Paragraph>,
}

impl Grid {
    fn close_cell(&mut self) {
        if let Some((mut cell, body)) = self.cell.take() {
            cell.text = body.finish();
            self.text.extend(cell.text.paragraphs.iter().cloned());
            self.row.push(cell);
        }
    }

    fn close_row(&mut self) {
        self.close_cell();
        if !self.row.is_empty() {
            self.rows.push(std::mem::take(&mut self.row));
        }
    }
}

pub(crate) fn read(html: &str) -> Option<Clip> {
    if html.len() > MAX_INPUT {
        return None;
    }
    let mut rules = Rules::new();
    let mut tokens = Tokens::new(html);
    let mut style_bytes = 0usize;
    let mut count = 0;
    while let Some(token) = tokens.next().ok()? {
        count += 1;
        if count > 200_000 {
            return None;
        }
        if let Token::Style(css) = token {
            style_bytes = style_bytes.checked_add(css.len())?;
            if style_bytes > MAX_STYLE {
                return None;
            }
            rules.add(css).ok()?;
        }
    }
    let mut tokens = Tokens::new(html);
    let mut frames: Vec<Frame> = vec![];
    let mut body = Body::default();
    let mut grid: Option<Grid> = None;
    let mut table = None;
    let mut tables = 0usize;
    let mut output_bytes = 0usize;
    while let Some(token) = tokens.next().ok()? {
        let parent = frames.last().map(|f| f.style.clone()).unwrap_or_default();
        let level = frames.iter().filter(|f| f.list.is_some()).count().saturating_sub(1).min(8) as u8;
        match token {
            Token::Text(text) => {
                let font_bytes = parent.run.font.as_ref().map_or(0, String::len);
                let link_bytes = parent.run.link.as_ref().map_or(0, |link| match &link.action {
                    Action::Url { url } => url.len(),
                    _ => 0,
                });
                output_bytes = output_bytes.checked_add(text.len() + font_bytes + link_bytes + std::mem::size_of::<Run>())?;
                if output_bytes > MAX_INPUT * 4 {
                    return None;
                }
                if let Some((_, cell_body)) = grid.as_mut().and_then(|g| g.cell.as_mut()) {
                    cell_body.text(text, &parent, level);
                } else if tables == 0 {
                    body.text(text, &parent, level);
                }
            }
            Token::Style(_) => {}
            Token::Open(tag) => {
                if frames.len() >= 64 && !tag.empty {
                    return None;
                }
                let mut style = if allowed(&tag.name) { rules.resolve(&tag, &parent) } else { parent };
                if tag.name == "a" {
                    // A rejected nested link must not inherit a safe parent's destination.
                    style.run.link = tag.attrs.get("href").filter(|href| safe_link(href)).map(|href| Hyperlink {
                        action: Action::Url { url: href.trim().to_owned() },
                        tooltip: String::new(),
                        highlight_click: false,
                    });
                }
                if tag.name == "li" {
                    if let Some(frame) = frames.iter_mut().rev().find(|f| f.list.is_some()) {
                        style.para.bullet.clone_from(&frame.list);
                        if let Some(Bullet::AutoNum { start_at, .. }) = &mut frame.list {
                            *start_at = start_at.saturating_add(1);
                        }
                    }
                    style.para.margin_left = Some(18.0 * (level as f64 + 1.0));
                    style.para.indent = Some(-12.0);
                }
                if tag.name == "table" {
                    if tables > 0 {
                        return None;
                    }
                    tables += 1;
                    body.flush(false);
                    if tables == 1 && table.is_none() {
                        grid = Some(Grid::default());
                    }
                }
                if tables == 1
                    && let Some(g) = &mut grid
                {
                    match tag.name.as_str() {
                        "tr" => g.close_row(),
                        "td" | "th" => {
                            g.close_cell();
                            g.count += 1;
                            if g.count > MAX_CELLS {
                                return None;
                            }
                            let span = |name: &str| -> Option<u32> {
                                let n = tag.attrs.get(name).map(|s| s.parse::<u32>().ok()).unwrap_or(Some(1))?.max(1);
                                (n as usize <= MAX_CELLS).then_some(n)
                            };
                            let cell = Cell {
                                grid_span: span("colspan")?,
                                row_span: span("rowspan")?,
                                fill: style.run.highlight.clone().map(Fill::solid),
                                ..Cell::new("")
                            };
                            g.cell = Some((cell, Body::default()));
                        }
                        _ => {}
                    }
                }
                let dest = if let Some((_, b)) = grid.as_mut().and_then(|g| g.cell.as_mut()) { b } else { &mut body };
                if block(&tag.name) {
                    dest.start(&style, level);
                }
                if tag.name == "br" {
                    dest.line_break(&style, level);
                }
                let list = match tag.name.as_str() {
                    "ul" => Some(Bullet::Char { char: "\u{2022}".into() }),
                    "ol" => Some(Bullet::AutoNum {
                        scheme: "arabicPeriod".into(),
                        start_at: tag.attrs.get("start").and_then(|s| s.parse().ok()).unwrap_or(1),
                    }),
                    _ => None,
                };
                if !tag.empty {
                    frames.push(Frame { name: tag.name, style, list });
                }
            }
            Token::Close(name) => {
                if block(&name) || matches!(name.as_str(), "ul" | "ol") {
                    let dest = if let Some((_, b)) = grid.as_mut().and_then(|g| g.cell.as_mut()) { b } else { &mut body };
                    dest.flush(block(&name));
                }
                if tables == 1
                    && let Some(g) = &mut grid
                {
                    if matches!(name.as_str(), "td" | "th") {
                        g.close_cell();
                    }
                    if name == "tr" {
                        g.close_row();
                    }
                }
                if name == "table" && tables > 0 {
                    tables -= 1;
                    if tables == 0
                        && let Some(mut g) = grid.take()
                    {
                        g.close_row();
                        body.paragraphs.extend(g.text);
                        table = Some(table_paste::from_rows(g.rows)?);
                    }
                }
                if let Some(at) = frames.iter().rposition(|f| f.name == name) {
                    frames.truncate(at);
                }
            }
        }
    }
    if let Some(mut g) = grid {
        g.close_row();
        body.paragraphs.extend(g.text);
        table = Some(table_paste::from_rows(g.rows)?);
    }
    let mut text = body.finish();
    if let Some(table) = &table {
        let (plain, _) = crate::clipboard_html::table(table, None);
        if text.is_empty() {
            text = TextBody::from_text(&plain);
        }
    }
    Some(Clip { text, table })
}

fn allowed(name: &str) -> bool {
    block(name)
        || matches!(
            name,
            "br" | "b" | "strong" | "i" | "em" | "u" | "s" | "strike" | "span" | "ul" | "ol" | "a" | "table" | "tr" | "td" | "th" | "body"
        )
}

fn block(name: &str) -> bool {
    matches!(name, "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li")
}

fn safe_link(href: &str) -> bool {
    let href = href.trim();
    !href.chars().any(char::is_control)
        && href.split_once(':').is_some_and(|(scheme, _)| matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https" | "mailto"))
}

pub(crate) fn plain(html: &str) -> String {
    // The system's plain flavor is preferred; this handles HTML-only failed imports.
    let end = html.char_indices().map(|(i, _)| i).take_while(|i| *i <= MAX_INPUT).last().unwrap_or(0);
    let bounded = if html.len() <= MAX_INPUT { html } else { html.get(..end).unwrap_or_default() };
    let mut out = String::new();
    let mut tokens = Tokens::plain(bounded);
    while let Ok(Some(token)) = tokens.next() {
        match token {
            Token::Text(text) => out.push_str(&decode(text)),
            Token::Open(tag) if tag.name == "br" => out.push('\n'),
            Token::Close(name) if block(&name) || name == "tr" => out.push('\n'),
            Token::Close(name) if name == "td" || name == "th" => out.push('\t'),
            _ => {}
        }
    }
    out.trim_end_matches(['\n', '\t']).to_owned()
}
