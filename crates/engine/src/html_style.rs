//! Clipboard CSS allow-list; values never resolve URLs or load resources.

use deckcraft_color::Rgba;
use deckcraft_model::text::{Align, ParaProps, RunProps, Strike};
use deckcraft_model::{ColorRef, Fill};

use crate::html_tokens::{MAX_STYLE, Tag};

#[derive(Clone, Default)]
pub(super) struct Style {
    pub run: RunProps,
    pub para: ParaProps,
}

pub(super) struct Rules(Vec<(String, String)>);

impl Rules {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn add(&mut self, css: &str) -> Result<(), ()> {
        let mut rest = css;
        while let Some(open) = rest.find('{') {
            let selectors = rest.get(..open).ok_or(())?.trim();
            let tail = rest.get(open + 1..).ok_or(())?;
            let close = tail.find('}').ok_or(())?;
            let declarations = tail.get(..close).ok_or(())?;
            for selector in selectors.split(',').map(str::trim) {
                let ident = selector.strip_prefix('.').unwrap_or(selector);
                if !ident.is_empty() && ident.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')) {
                    if self.0.len() >= 256 {
                        return Err(());
                    }
                    self.0.push((selector.to_owned(), declarations.to_owned()));
                }
            }
            rest = tail.get(close + 1..).ok_or(())?;
        }
        Ok(())
    }

    pub fn resolve(&self, tag: &Tag, parent: &Style) -> Style {
        let mut style = parent.clone();
        match tag.name.as_str() {
            "b" | "strong" | "th" => style.run.bold = Some(true),
            "i" | "em" => style.run.italic = Some(true),
            "u" => style.run.underline = Some("sng".into()),
            "s" | "strike" => style.run.strike = Some(Strike::Single),
            "h1" => style.run.size = Some(32.0),
            "h2" => style.run.size = Some(24.0),
            "h3" => style.run.size = Some(20.0),
            "h4" => style.run.size = Some(18.0),
            "h5" => style.run.size = Some(14.0),
            "h6" => style.run.size = Some(12.0),
            _ => {}
        }
        if matches!(tag.name.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
            style.run.bold = Some(true);
        }
        for (selector, css) in &self.0 {
            if selector.eq_ignore_ascii_case(&tag.name) {
                style.apply(css);
            }
        }
        for (selector, css) in &self.0 {
            if let Some(class) = selector.strip_prefix('.')
                && tag.attrs.get("class").is_some_and(|s| s.split_whitespace().any(|c| c == class))
            {
                style.apply(css);
            }
        }
        if let Some(css) = tag.attrs.get("style") {
            style.apply(css);
        }
        style
    }
}

impl Style {
    pub fn apply(&mut self, css: &str) {
        if css.len() > MAX_STYLE {
            return;
        }
        for declaration in css.split(';') {
            let Some((key, value)) = declaration.split_once(':') else { continue };
            let value = value.trim().trim_end_matches("!important").trim();
            let lower = value.to_ascii_lowercase();
            match key.trim().to_ascii_lowercase().as_str() {
                "color" => {
                    if let Some(color) = color(&lower) {
                        self.run.fill = Some(Fill::solid(color));
                    }
                }
                "background-color" => {
                    if let Some(color) = color(&lower) {
                        self.run.highlight = Some(color);
                    }
                }
                "font-size" => {
                    let points = lower
                        .strip_suffix("pt")
                        .and_then(|n| n.trim().parse::<f64>().ok())
                        .or_else(|| lower.strip_suffix("px").and_then(|n| n.trim().parse::<f64>().ok()).map(|n| n * 0.75));
                    if let Some(size) = points.filter(|n| n.is_finite() && *n > 0.0 && *n <= 1000.0) {
                        self.run.size = Some(size);
                    }
                }
                "font-family" => {
                    let family = value.split(',').next().unwrap_or_default().trim().trim_matches(['\'', '"']);
                    if !family.is_empty() && !family.contains(['(', ')', '\\']) {
                        self.run.font = Some(family.to_owned());
                    }
                }
                "font-weight" => {
                    if matches!(lower.as_str(), "bold" | "bolder") {
                        self.run.bold = Some(true);
                    } else if lower == "normal" {
                        self.run.bold = Some(false);
                    } else if let Ok(weight) = lower.parse::<u16>() {
                        self.run.bold = Some(weight >= 600);
                    }
                }
                "font-style" => match lower.as_str() {
                    "italic" | "oblique" => self.run.italic = Some(true),
                    "normal" => self.run.italic = Some(false),
                    _ => {}
                },
                "text-decoration" | "text-decoration-line" => {
                    for part in lower.split_whitespace() {
                        match part {
                            "underline" => self.run.underline = Some("sng".into()),
                            "line-through" => self.run.strike = Some(Strike::Single),
                            "none" => {
                                self.run.underline = Some("none".into());
                                self.run.strike = Some(Strike::None);
                            }
                            _ => {}
                        }
                    }
                }
                "text-align" => {
                    if let Some(align) = match lower.as_str() {
                        "left" => Some(Align::Left),
                        "center" => Some(Align::Center),
                        "right" => Some(Align::Right),
                        "justify" => Some(Align::Justify),
                        _ => None,
                    } {
                        self.para.align = Some(align);
                    }
                }
                _ => {}
            }
        }
    }
}

fn color(value: &str) -> Option<ColorRef> {
    let rgba = if value.starts_with('#') {
        Rgba::from_hex(value)?
    } else if let Some(rgb) = value.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let mut parts = rgb.split(',');
        let mut channel = || parts.next()?.trim().parse::<u8>().ok();
        let rgba = Rgba::rgb(channel()?, channel()?, channel()?);
        if parts.next().is_some() {
            return None;
        }
        rgba
    } else {
        match value {
            "transparent" => Rgba::TRANSPARENT,
            "grey" => Rgba::rgb(128, 128, 128),
            _ => deckcraft_color::preset(value)?,
        }
    };
    Some(ColorRef::rgb(rgba))
}
