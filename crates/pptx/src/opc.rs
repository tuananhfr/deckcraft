//! Open Packaging Conventions: the zip container, part names, relationships and content types.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Write};

use deckcraft_model::inflate::{InflateError, Inflater, LIMITS, Limits};

use crate::PptxError;
use crate::xml::{self, A, W};

/// Largest XML part we parse.
pub const MAX_XML_PART: usize = 96 * 1024 * 1024;

pub const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
pub const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
pub const NS_C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
pub const NS_MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
pub const NS_P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";
pub const NS_P15: &str = "http://schemas.microsoft.com/office/powerpoint/2012/main";
pub const NS_P159: &str = "http://schemas.microsoft.com/office/powerpoint/2015/09/main";
pub const NS_A14: &str = "http://schemas.microsoft.com/office/drawing/2010/main";

pub const RT_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
pub const RT_MEDIA: &str = "http://schemas.microsoft.com/office/2007/relationships/media";
pub const RT_CORE: &str = "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";

/// The files of a package, keyed by normalised part name (no leading slash, lowercase).
pub struct Package {
    parts: HashMap<String, (String, Vec<u8>)>,
    read_parts: RefCell<HashSet<String>>,
}

impl Package {
    pub fn open(bytes: &[u8]) -> Result<Self, PptxError> {
        Package::open_with(bytes, LIMITS)
    }

    /// [`Package::open`] under other limits (smaller ones in tests).
    pub fn open_with(bytes: &[u8], limits: Limits) -> Result<Self, PptxError> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| PptxError::NotPptx(format!("{e}")))?;
        let mut inflater = Inflater::new(limits);
        inflater.check_entries(zip.len()).map_err(|e| PptxError::TooLarge(e.to_string()))?;
        let mut parts = HashMap::new();
        for i in 0..zip.len() {
            let f = match zip.by_index(i) {
                Ok(f) => f,
                Err(e) => {
                    log::warn!("pptx: skipping unreadable zip entry {i}: {e}");
                    continue;
                }
            };
            if f.is_dir() {
                continue;
            }
            let name = f.name().trim_start_matches('/').to_string();
            let (declared, compressed) = (f.size(), f.compressed_size());
            let data = match inflater.read(&name, f, declared, compressed, u64::MAX) {
                Ok(data) => data,
                // Over only a caller limit: just this part goes.
                Err(InflateError::TooBig(e)) => {
                    log::warn!("pptx: skipping oversized part {e}");
                    continue;
                }
                Err(InflateError::Damaged(e)) => {
                    log::warn!("pptx: could not read {e}");
                    continue;
                }
                Err(InflateError::Refused(e)) => return Err(PptxError::TooLarge(e)),
            };
            parts.insert(name.to_ascii_lowercase(), (name, data));
        }
        Ok(Package { parts, read_parts: RefCell::new(HashSet::new()) })
    }
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        let key = norm(name);
        let (_, data) = self.parts.get(&key)?;
        if let Ok(mut read) = self.read_parts.try_borrow_mut() {
            read.insert(key);
        }
        Some(data.as_slice())
    }
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.parts.keys().map(String::as_str)
    }
    pub(crate) fn read_parts(&self) -> HashSet<String> {
        self.read_parts.try_borrow().map(|read| read.clone()).unwrap_or_default()
    }
    /// Parse an XML part.
    pub fn xml(&self, name: &str) -> Option<xml::Doc> {
        let b = self.get(name)?;
        if b.len() > MAX_XML_PART {
            log::warn!("pptx: XML part {name} too large");
            return None;
        }
        match xml::parse(b) {
            Ok(d) => Some(d),
            Err(e) => {
                log::warn!("pptx: {name}: {e}");
                None
            }
        }
    }
    /// Relationships of a part (`ppt/slides/slide1.xml` → `ppt/slides/_rels/slide1.xml.rels`).
    pub fn rels(&self, part: &str) -> Rels {
        let rp = rels_path(part);
        let Some(doc) = self.xml(&rp) else { return Rels::default() };
        let base = dir_of(part);
        let mut out = Rels::default();
        for r in doc.root.children_named("Relationship") {
            let (Some(id), Some(ty), Some(target)) = (r.attr("Id"), r.attr("Type"), r.attr("Target")) else { continue };
            let external = r.attr("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("external"));
            let target = if external { target.to_string() } else { resolve(&base, target) };
            out.items.push(Rel { id: id.to_string(), kind: ty.rsplit('/').next().unwrap_or(ty).to_string(), target, external });
        }
        out
    }
    /// Content types: (extension defaults, part overrides).
    pub fn content_type(&self, part: &str) -> Option<String> {
        let doc = self.xml("[Content_Types].xml")?;
        let p = format!("/{}", norm(part));
        for o in doc.root.children_named("Override") {
            if o.attr("PartName").is_some_and(|n| n.to_ascii_lowercase() == p) {
                return o.attr("ContentType").map(String::from);
            }
        }
        let ext = part.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        doc.root
            .children_named("Default")
            .find(|d| d.attr("Extension").is_some_and(|e| e.eq_ignore_ascii_case(&ext)))
            .and_then(|d| d.attr("ContentType").map(String::from))
    }
    pub fn original_name(&self, name: &str) -> Option<&str> {
        self.parts.get(&norm(name)).map(|(n, _)| n.as_str())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Rel {
    pub id: String,
    /// Last segment of the relationship type (`slide`, `image`, `media`…).
    pub kind: String,
    /// Absolute part name for internal targets; the URL for external ones.
    pub target: String,
    pub external: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Rels {
    pub items: Vec<Rel>,
}

impl Rels {
    pub fn get(&self, id: &str) -> Option<&Rel> {
        self.items.iter().find(|r| r.id == id)
    }
    pub fn target(&self, id: &str) -> Option<&str> {
        self.get(id).map(|r| r.target.as_str())
    }
    pub fn first_of(&self, kind: &str) -> Option<&Rel> {
        self.items.iter().find(|r| r.kind == kind)
    }
    pub fn all_of<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Rel> + 'a {
        self.items.iter().filter(move |r| r.kind == kind)
    }
}

pub fn norm(name: &str) -> String {
    name.trim_start_matches('/').replace('\\', "/").to_ascii_lowercase()
}

pub fn dir_of(part: &str) -> String {
    let p = part.trim_start_matches('/');
    match p.rfind('/') {
        Some(i) => p.get(..i).unwrap_or("").to_string(),
        None => String::new(),
    }
}

pub fn file_of(part: &str) -> &str {
    part.rsplit('/').next().unwrap_or(part)
}

pub fn rels_path(part: &str) -> String {
    let p = part.trim_start_matches('/');
    let dir = dir_of(p);
    let file = file_of(p);
    if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b.get(i).copied().unwrap_or(0);
        if c == b'%'
            && let Some(h) = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(h);
            i += 3;
            continue;
        }
        out.push(c);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolve a relative target against a base directory into a part name.
pub fn resolve(base: &str, target: &str) -> String {
    let target = percent_decode(target);
    let target = target.split('#').next().unwrap_or("");
    let mut segs: Vec<&str> = if target.starts_with('/') { vec![] } else { base.split('/').filter(|s| !s.is_empty()).collect() };
    for s in target.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    segs.join("/")
}

/// Relative target from the directory of `from` to part `to` (both absolute part names).
pub fn relative(from: &str, to: &str) -> String {
    let fd_s = dir_of(from);
    let fd: Vec<&str> = fd_s.split('/').filter(|s| !s.is_empty()).collect();
    let td_s = dir_of(to);
    let td: Vec<&str> = td_s.split('/').filter(|s| !s.is_empty()).collect();
    let common = fd.iter().zip(td.iter()).take_while(|(a, b)| a == b).count();
    let mut out = String::new();
    for _ in common..fd.len() {
        out.push_str("../");
    }
    for s in td.iter().skip(common) {
        out.push_str(s);
        out.push('/');
    }
    out.push_str(file_of(to));
    out
}

// ---------------------------------------------------------------------------------------------
// Writing

/// Relationships being built for one part.
#[derive(Default)]
pub struct RelsOut {
    pub items: Vec<(String, String, String, bool)>,
}

impl RelsOut {
    /// Add (or reuse) a relationship; `kind` is a full type URI or a suffix of the standard base.
    pub fn add(&mut self, kind: &str, target: &str, external: bool) -> String {
        let ty = if kind.starts_with("http") { kind.to_string() } else { format!("{RT_BASE}{kind}") };
        if let Some((id, ..)) = self.items.iter().find(|(_, t, tg, e)| *t == ty && tg == target && *e == external) {
            return id.clone();
        }
        let id = format!("rId{}", self.items.len() + 1);
        self.items.push((id.clone(), ty, target.to_string(), external));
        id
    }
    /// Always add a new relationship even if an identical one exists.
    pub fn add_new(&mut self, kind: &str, target: &str, external: bool) -> String {
        let ty = if kind.starts_with("http") { kind.to_string() } else { format!("{RT_BASE}{kind}") };
        let id = format!("rId{}", self.items.len() + 1);
        self.items.push((id.clone(), ty, target.to_string(), external));
        id
    }
    pub fn to_xml(&self) -> Vec<u8> {
        let mut w = W::new();
        w.open("Relationships", A::new().a("xmlns", "http://schemas.openxmlformats.org/package/2006/relationships"));
        for (id, ty, target, ext) in &self.items {
            let mut a = A::new().a("Id", id).a("Type", ty).a("Target", target);
            if *ext {
                a = a.a("TargetMode", "External");
            }
            w.empty("Relationship", a);
        }
        w.close("Relationships");
        w.finish()
    }
}

/// A package being written.
pub struct PackageOut {
    files: Vec<(String, Vec<u8>, bool)>,
    defaults: Vec<(String, String)>,
    overrides: Vec<(String, String)>,
}

impl Default for PackageOut {
    fn default() -> Self {
        PackageOut {
            files: vec![],
            defaults: vec![
                ("rels".into(), "application/vnd.openxmlformats-package.relationships+xml".into()),
                ("xml".into(), "application/xml".into()),
            ],
            overrides: vec![],
        }
    }
}

impl PackageOut {
    /// Add an XML part with its content type override.
    pub fn xml_part(&mut self, name: &str, content_type: &str, data: Vec<u8>) {
        self.overrides.push((format!("/{name}"), content_type.to_string()));
        self.files.push((name.to_string(), data, true));
    }
    pub fn rels(&mut self, part: &str, rels: &RelsOut) {
        if rels.items.is_empty() {
            return;
        }
        self.files.push((rels_path(part), rels.to_xml(), true));
    }
    /// Add a binary part; its extension gets a Default content type.
    pub fn binary(&mut self, name: &str, content_type: &str, data: Vec<u8>) {
        let ext = name.rsplit('.').next().unwrap_or("bin").to_ascii_lowercase();
        if !self.defaults.iter().any(|(e, _)| *e == ext) {
            self.defaults.push((ext, content_type.to_string()));
        } else if !self.defaults.iter().any(|(e, c)| *e == ext && c == content_type) {
            self.overrides.push((format!("/{name}"), content_type.to_string()));
        }
        self.files.push((name.to_string(), data, false));
    }
    pub fn finish(self) -> Result<Vec<u8>, PptxError> {
        let mut ct = W::new();
        ct.open("Types", A::new().a("xmlns", "http://schemas.openxmlformats.org/package/2006/content-types"));
        for (e, c) in &self.defaults {
            ct.empty("Default", A::new().a("Extension", e).a("ContentType", c));
        }
        for (p, c) in &self.overrides {
            ct.empty("Override", A::new().a("PartName", p).a("ContentType", c));
        }
        ct.close("Types");
        let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let deflate = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let err = |e: &dyn std::fmt::Display| PptxError::Write(e.to_string());
        zw.start_file("[Content_Types].xml", deflate).map_err(|e| err(&e))?;
        zw.write_all(&ct.finish()).map_err(|e| err(&e))?;
        for (name, data, compress) in &self.files {
            let large = data.len() as u64 >= u32::MAX as u64 / 2;
            let opts = if *compress { deflate } else { stored }.large_file(large);
            zw.start_file(name.as_str(), opts).map_err(|e| err(&e))?;
            zw.write_all(data).map_err(|e| err(&e))?;
        }
        let cur = zw.finish().map_err(|e| err(&e))?;
        Ok(cur.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (n, d) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(d).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    /// Bytes deflate barely compresses (xorshift), so only the size caps can stop them.
    fn noise(len: usize, mut x: u64) -> Vec<u8> {
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    }

    /// Rewrites the inflated size `name` declares, in its local header and the central directory.
    fn declare_size(zip: &mut [u8], name: &str, size: u32) {
        for i in 0..zip.len() - 4 {
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

    fn small(entry: u64, total: u64) -> Limits {
        Limits { entry, total, ..LIMITS }
    }

    #[test]
    fn a_declared_small_part_is_cut_off_while_it_inflates() {
        let mut bytes = zip_of(&[("ppt/media/big.bin", &noise(3 * MIB as usize, 7))]);
        declare_size(&mut bytes, "ppt/media/big.bin", 100);
        assert!(matches!(Package::open_with(&bytes, small(MIB, 64 * MIB)), Err(PptxError::TooLarge(m)) if m.contains("inflates to more than")));
        // Declared over the cap, the package is refused without inflating the part.
        let bytes = zip_of(&[("ppt/media/big.bin", &noise(3 * MIB as usize, 7)), ("ppt/a.xml", b"<a/>")]);
        assert!(matches!(Package::open_with(&bytes, small(MIB, 64 * MIB)), Err(PptxError::TooLarge(m)) if m.contains("declares")));
    }

    #[test]
    fn the_package_total_is_capped() {
        let honest = zip_of(&[("a.bin", &noise(2 * MIB as usize, 3)), ("b.bin", &noise(2 * MIB as usize, 5))]);
        assert!(Package::open_with(&honest, small(4 * MIB, 5 * MIB)).is_ok());
        let mut bytes = honest;
        declare_size(&mut bytes, "b.bin", 100);
        assert!(matches!(Package::open_with(&bytes, small(4 * MIB, 3 * MIB)), Err(PptxError::TooLarge(m)) if m.contains("package")));
    }

    #[test]
    fn path_resolution() {
        assert_eq!(resolve("ppt/slides", "../media/image1.png"), "ppt/media/image1.png");
        assert_eq!(resolve("ppt", "slides/slide1.xml"), "ppt/slides/slide1.xml");
        assert_eq!(resolve("ppt/slides", "/ppt/media/a%20b.png"), "ppt/media/a b.png");
        assert_eq!(resolve("", "../../../x.xml"), "x.xml");
        assert_eq!(rels_path("ppt/presentation.xml"), "ppt/_rels/presentation.xml.rels");
        assert_eq!(rels_path("[Content_Types].xml"), "_rels/[Content_Types].xml.rels");
        assert_eq!(relative("ppt/slides/slide1.xml", "ppt/media/image1.png"), "../media/image1.png");
        assert_eq!(relative("ppt/presentation.xml", "ppt/slides/slide1.xml"), "slides/slide1.xml");
        assert_eq!(relative("ppt/slides/slide1.xml", "ppt/slides/slide2.xml"), "slide2.xml");
    }
}
