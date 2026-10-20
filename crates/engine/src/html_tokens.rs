//! Bounded, non-recursive tokenization of clipboard HTML fragments.

use std::collections::BTreeMap;

pub(super) const MAX_INPUT: usize = 8 * 1024 * 1024;
pub(super) const MAX_STYLE: usize = 8 * 1024;

pub(super) struct Tag {
    pub name: String,
    pub attrs: BTreeMap<String, String>,
    pub empty: bool,
}

pub(super) enum Token<'a> {
    Open(Tag),
    Close(String),
    Text(&'a str),
    Style(&'a str),
}

pub(super) struct Tokens<'a> {
    rest: &'a str,
    plain: bool,
}

impl<'a> Tokens<'a> {
    pub fn new(html: &'a str) -> Self {
        Self { rest: html, plain: false }
    }

    pub fn plain(html: &'a str) -> Self {
        Self { rest: html, plain: true }
    }

    pub fn next(&mut self) -> Result<Option<Token<'a>>, ()> {
        loop {
            if self.rest.is_empty() {
                return Ok(None);
            }
            if !self.rest.starts_with('<') {
                let n = self.rest.find('<').unwrap_or(self.rest.len());
                let text = self.rest.get(..n).ok_or(())?;
                self.rest = self.rest.get(n..).ok_or(())?;
                return Ok(Some(Token::Text(text)));
            }
            if self.rest.starts_with("<!--") {
                let n = self.rest.find("-->").map(|n| n + 3).unwrap_or(self.rest.len());
                self.rest = self.rest.get(n..).ok_or(())?;
                continue;
            }
            if !self.rest.as_bytes().get(1).is_some_and(|b| b.is_ascii_alphabetic() || matches!(b, b'/' | b'!' | b'?')) {
                self.rest = self.rest.get(1..).ok_or(())?;
                return Ok(Some(Token::Text("<")));
            }
            let mut quote = None;
            let mut end = None;
            for (i, b) in self.rest.bytes().enumerate().skip(1) {
                if !self.plain && i > MAX_STYLE {
                    return Err(());
                }
                match (quote, b) {
                    (Some(q), b) if q == b => quote = None,
                    (None, b'\'' | b'"') => quote = Some(b),
                    (None, b'>') => {
                        end = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            let Some(end) = end else {
                let text = self.rest;
                self.rest = "";
                return Ok(Some(Token::Text(text)));
            };
            let raw = self.rest.get(1..end).ok_or(())?.trim();
            self.rest = self.rest.get(end + 1..).ok_or(())?;
            if raw.starts_with(['!', '?']) {
                continue;
            }
            let closing = raw.starts_with('/');
            let raw = raw.trim_start_matches('/').trim_start();
            let n = raw.bytes().take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b':')).count();
            let name = raw.get(..n).ok_or(())?.to_ascii_lowercase();
            if closing {
                return Ok(Some(Token::Close(name)));
            }
            if matches!(name.as_str(), "script" | "style") {
                let needle = format!("</{name}");
                let at = self.rest.as_bytes().windows(needle.len()).enumerate().find_map(|(i, w)| {
                    (w.eq_ignore_ascii_case(needle.as_bytes())
                        && self.rest.as_bytes().get(i + needle.len()).is_some_and(|b| b.is_ascii_whitespace() || *b == b'>'))
                    .then_some(i)
                });
                let body = self.rest.get(..at.unwrap_or(self.rest.len())).ok_or(())?;
                self.rest = at.and_then(|i| self.rest.get(i..)).and_then(|s| s.find('>').and_then(|n| s.get(n + 1..))).unwrap_or("");
                if name == "style" {
                    if !self.plain && body.len() > MAX_STYLE {
                        return Err(());
                    }
                    return Ok(Some(Token::Style(body)));
                }
                continue;
            }
            let attrs = if self.plain { BTreeMap::new() } else { attributes(raw.get(n..).ok_or(())?)? };
            let empty = raw.ends_with('/')
                || matches!(
                    name.as_str(),
                    "br" | "hr" | "img" | "meta" | "link" | "input" | "wbr" | "col" | "area" | "base" | "embed" | "source" | "track" | "param"
                );
            return Ok(Some(Token::Open(Tag { name, attrs, empty })));
        }
    }
}

fn attributes(mut raw: &str) -> Result<BTreeMap<String, String>, ()> {
    let mut attrs = BTreeMap::new();
    while !raw.trim_start().is_empty() {
        raw = raw.trim_start();
        let n = raw.bytes().take_while(|b| !b.is_ascii_whitespace() && !matches!(b, b'=' | b'/')).count();
        if n == 0 {
            raw = raw.get(1..).ok_or(())?;
            continue;
        }
        let name = raw.get(..n).ok_or(())?.to_ascii_lowercase();
        raw = raw.get(n..).ok_or(())?.trim_start();
        if !raw.starts_with('=') {
            continue;
        }
        raw = raw.get(1..).ok_or(())?.trim_start();
        let (value, rest) = match raw.as_bytes().first() {
            Some(b'\'' | b'"') => {
                let quote = raw.as_bytes().first().copied().ok_or(())? as char;
                let tail = raw.get(1..).ok_or(())?;
                let n = tail.find(quote).unwrap_or(tail.len());
                (tail.get(..n).ok_or(())?, tail.get((n + 1).min(tail.len())..).ok_or(())?)
            }
            _ => {
                let n = raw.find(char::is_whitespace).unwrap_or(raw.len());
                (raw.get(..n).ok_or(())?, raw.get(n..).ok_or(())?)
            }
        };
        if matches!(name.as_str(), "style" | "class" | "href" | "colspan" | "rowspan" | "start") {
            attrs.insert(name, decode(value));
        }
        raw = rest;
    }
    Ok(attrs)
}

pub(super) fn decode(mut text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    while let Some(at) = text.find('&') {
        out.push_str(text.get(..at).unwrap_or_default());
        let tail = text.get(at + 1..).unwrap_or_default();
        let end = tail.bytes().take(32).position(|b| b == b';');
        let value = end.and_then(|n| tail.get(..n)).and_then(entity);
        if let (Some(n), Some(ch)) = (end, value) {
            out.push(ch);
            text = tail.get(n + 1..).unwrap_or_default();
        } else {
            out.push('&');
            text = tail;
        }
    }
    out.push_str(text);
    out
}

fn entity(s: &str) -> Option<char> {
    Some(match s {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "bull" => '\u{2022}',
        "ndash" => '\u{2013}',
        "mdash" => '\u{2014}',
        "hellip" => '\u{2026}',
        "lsquo" => '\u{2018}',
        "rsquo" => '\u{2019}',
        "ldquo" => '\u{201c}',
        "rdquo" => '\u{201d}',
        "copy" => '\u{a9}',
        "reg" => '\u{ae}',
        "trade" => '\u{2122}',
        "euro" => '\u{20ac}',
        _ => {
            let n = if let Some(hex) = s.strip_prefix("#x").or_else(|| s.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                s.strip_prefix('#')?.parse().ok()?
            };
            char::from_u32(n)?
        }
    })
}
