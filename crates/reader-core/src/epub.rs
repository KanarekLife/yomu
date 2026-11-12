//! Minimal, tolerant EPUB 2/3 reader (ADR-0006).
//!
//! We only need: metadata (title/author), the cover image, the spine in
//! reading order, chapter titles from the NCX / nav document, and the text of
//! each spine document split into paragraphs with author-provided ruby.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use scraper::{ElementRef, Html, Node};

use crate::segment::{Paragraph, RubySpan};
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct ParsedChapter {
    pub title: String,
    pub paragraphs: Vec<Paragraph>,
}

#[derive(Debug, Clone)]
pub struct ParsedBook {
    pub title: String,
    pub author: String,
    pub language: String,
    pub cover: Option<(Vec<u8>, String)>,
    pub chapters: Vec<ParsedChapter>,
}

struct Archive {
    zip: zip::ZipArchive<Cursor<Vec<u8>>>,
}

impl Archive {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        let mut f = self.zip.by_name(path).ok().or(None)?;
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut buf).ok()?;
        Some(buf)
    }

    fn read_string(&mut self, path: &str) -> Option<String> {
        self.read(path).map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// Case-insensitive fallback lookup – some EPUBs disagree with
    /// themselves about file name casing.
    fn read_any(&mut self, path: &str) -> Option<Vec<u8>> {
        if let Some(b) = self.read(path) {
            return Some(b);
        }
        let lower = path.to_lowercase();
        let name = self.zip.file_names().filter_map(|n| n.ok()).find(|n| n.to_lowercase() == lower)?.into_owned();
        self.read(&name)
    }
}

/// Read one file from an EPUB (case-insensitive fallback).
pub fn read_entry(epub: &[u8], path: &str) -> Option<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(epub)).ok()?;
    let name = match zip.index_for_name(path) {
        Some(_) => path.to_string(),
        None => {
            let lower = path.to_lowercase();
            zip.file_names().filter_map(|n| n.ok()).find(|n| n.to_lowercase() == lower)?.into_owned()
        }
    };
    let mut f = zip.by_name(&name).ok()?;
    let mut buf = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolve `href` relative to the directory `base_dir` (no leading slash,
/// may be empty). Strips fragments and normalises `.`/`..`.
fn resolve(base_dir: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or("");
    let href = percent_decode(href);
    let mut parts: Vec<&str> = if href.starts_with('/') {
        Vec::new()
    } else {
        base_dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn dir_of(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..i].to_string(),
        None => String::new(),
    }
}

fn local<'a>(n: &roxmltree::Node<'a, 'a>) -> &'a str {
    n.tag_name().name()
}

struct ManifestItem {
    href: String,
    media_type: String,
    properties: String,
}

pub fn parse_epub(bytes: Vec<u8>) -> Result<ParsedBook> {
    let zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::Epub(format!("not a zip archive: {e}")))?;
    let mut ar = Archive { zip };

    // 1. container.xml → OPF path
    let container = ar
        .read_string("META-INF/container.xml")
        .ok_or_else(|| Error::Epub("missing META-INF/container.xml".into()))?;
    let cdoc = roxmltree::Document::parse(&container).map_err(|e| Error::Epub(format!("container.xml: {e}")))?;
    let opf_path = cdoc
        .descendants()
        .find(|n| local(n) == "rootfile")
        .and_then(|n| n.attribute("full-path"))
        .ok_or_else(|| Error::Epub("no rootfile in container.xml".into()))?
        .to_string();
    let opf_dir = dir_of(&opf_path);

    // 2. OPF
    let opf = ar.read_any(&opf_path).ok_or_else(|| Error::Epub(format!("missing {opf_path}")))?;
    let opf = String::from_utf8_lossy(&opf).into_owned();
    let opt = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let odoc = roxmltree::Document::parse_with_options(&opf, opt).map_err(|e| Error::Epub(format!("OPF: {e}")))?;

    let mut title = String::new();
    let mut author = String::new();
    let mut language = String::new();
    let mut cover_id: Option<String> = None;
    let mut manifest: HashMap<String, ManifestItem> = HashMap::new();
    let mut spine: Vec<String> = Vec::new();
    let mut ncx_id: Option<String> = None;

    for n in odoc.descendants().filter(|n| n.is_element()) {
        match local(&n) {
            "title" if title.is_empty() => title = n.text().unwrap_or("").trim().to_string(),
            "creator" if author.is_empty() => author = n.text().unwrap_or("").trim().to_string(),
            "language" if language.is_empty() => language = n.text().unwrap_or("").trim().to_string(),
            "meta" if n.attribute("name") == Some("cover") => cover_id = n.attribute("content").map(String::from),
            "item" => {
                if let (Some(id), Some(href)) = (n.attribute("id"), n.attribute("href")) {
                    manifest.insert(
                        id.to_string(),
                        ManifestItem {
                            href: resolve(&opf_dir, href),
                            media_type: n.attribute("media-type").unwrap_or("").to_string(),
                            properties: n.attribute("properties").unwrap_or("").to_string(),
                        },
                    );
                }
            }
            "spine" => ncx_id = n.attribute("toc").map(String::from),
            "itemref" => {
                if n.attribute("linear") != Some("no") {
                    if let Some(id) = n.attribute("idref") {
                        spine.push(id.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    // 3. cover
    let cover_item = cover_id
        .as_ref()
        .and_then(|id| manifest.get(id))
        .filter(|m| m.media_type.starts_with("image/"))
        .or_else(|| manifest.values().find(|m| m.properties.split_whitespace().any(|p| p == "cover-image")))
        .or_else(|| {
            manifest
                .iter()
                .find(|(id, m)| m.media_type.starts_with("image/") && (id.to_lowercase().contains("cover") || m.href.to_lowercase().contains("cover")))
                .map(|(_, m)| m)
        });
    let cover = cover_item.and_then(|m| {
        let mime = m.media_type.clone();
        ar.read_any(&m.href).map(|b| (b, mime))
    });

    // 4. TOC: file path → title (first entry wins)
    let mut toc: Vec<(String, String)> = Vec::new();
    if let Some(nav) = manifest.values().find(|m| m.properties.split_whitespace().any(|p| p == "nav")) {
        let nav_href = nav.href.clone();
        if let Some(s) = ar.read_any(&nav_href) {
            toc = parse_nav(&String::from_utf8_lossy(&s), &dir_of(&nav_href));
        }
    }
    if toc.is_empty() {
        let ncx = ncx_id
            .as_ref()
            .and_then(|id| manifest.get(id))
            .or_else(|| manifest.values().find(|m| m.media_type == "application/x-dtbncx+xml"))
            .map(|m| m.href.clone());
        if let Some(ncx_href) = ncx {
            if let Some(s) = ar.read_any(&ncx_href) {
                toc = parse_ncx(&String::from_utf8_lossy(&s), &dir_of(&ncx_href));
            }
        }
    }
    let mut toc_map: HashMap<String, String> = HashMap::new();
    for (href, t) in &toc {
        toc_map.entry(href.clone()).or_insert_with(|| t.clone());
    }

    // 5. Spine documents
    let mut chapters: Vec<ParsedChapter> = Vec::new();
    for (si, id) in spine.iter().enumerate() {
        let Some(item) = manifest.get(id) else { continue };
        if !(item.media_type.contains("html") || item.media_type.contains("xml")) {
            continue;
        }
        let Some(raw) = ar.read_any(&item.href) else { continue };
        let html = String::from_utf8_lossy(&raw);
        let (paras, heading) = extract_paragraphs(&html, &dir_of(&item.href));
        let toc_title = toc_map.get(&item.href).cloned();
        match (toc_title, chapters.last_mut()) {
            // Start a new chapter when the TOC points here, or when there is no
            // TOC at all and the document is non-empty.
            (Some(t), _) => chapters.push(ParsedChapter { title: t, paragraphs: paras }),
            (None, Some(last)) if !toc.is_empty() => last.paragraphs.extend(paras),
            (None, _) => {
                if paras.is_empty() && chapters.is_empty() {
                    continue;
                }
                if toc.is_empty() {
                    let t = heading.unwrap_or_else(|| format!("§{}", si + 1));
                    chapters.push(ParsedChapter { title: t, paragraphs: paras });
                } else {
                    // Front matter before the first TOC entry.
                    chapters.push(ParsedChapter { title: title.clone(), paragraphs: paras });
                }
            }
        }
    }
    chapters.retain(|c| !c.paragraphs.is_empty());

    if title.is_empty() {
        title = "Untitled".into();
    }
    Ok(ParsedBook { title, author, language, cover, chapters })
}

fn parse_ncx(xml: &str, base: &str) -> Vec<(String, String)> {
    let opt = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let Ok(doc) = roxmltree::Document::parse_with_options(xml, opt) else { return vec![] };
    let mut out = Vec::new();
    for np in doc.descendants().filter(|n| local(n) == "navPoint") {
        let label = np
            .children()
            .find(|c| local(c) == "navLabel")
            .and_then(|l| l.descendants().find(|c| local(c) == "text"))
            .and_then(|t| t.text())
            .unwrap_or("")
            .trim()
            .to_string();
        let src = np.children().find(|c| local(c) == "content").and_then(|c| c.attribute("src"));
        if let Some(src) = src {
            out.push((resolve(base, src), label));
        }
    }
    out
}

fn parse_nav(html: &str, base: &str) -> Vec<(String, String)> {
    let doc = Html::parse_document(html);
    let nav_sel = scraper::Selector::parse("nav").unwrap();
    let a_sel = scraper::Selector::parse("a[href]").unwrap();
    let navs: Vec<ElementRef> = doc.select(&nav_sel).collect();
    let toc_nav = navs
        .iter()
        .find(|n| n.value().attrs().any(|(k, v)| k.ends_with("type") && v.split_whitespace().any(|t| t == "toc")))
        .or(navs.first());
    let Some(nav) = toc_nav else { return vec![] };
    nav.select(&a_sel)
        .filter_map(|a| {
            let href = a.value().attr("href")?;
            let label: String = a.text().collect::<String>().trim().to_string();
            Some((resolve(base, href), label))
        })
        .collect()
}

const BLOCKS: &[&str] = &[
    "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "blockquote", "section", "article", "tr", "td", "th", "dt",
    "dd", "figcaption", "header", "footer", "aside", "pre", "table", "ul", "ol", "body", "hr",
];
const SKIP: &[&str] = &["rt", "rp", "script", "style", "head", "title", "svg", "math", "nav"];

struct Collector<'b> {
    base: &'b str,
    in_heading: bool,
    paras: Vec<Paragraph>,
    cur: Paragraph,
    cur_len: usize,
    heading: Option<String>,
}

impl Collector<'_> {
    fn push_text(&mut self, s: &str) {
        for c in s.chars() {
            if matches!(c, '\n' | '\r' | '\t') {
                continue;
            }
            self.cur.text.push(c);
            self.cur_len += 1;
        }
    }
    fn flush(&mut self) {
        if self.cur.text.chars().any(|c| !(c.is_whitespace() || c == '\u{3000}')) {
            self.cur.heading = self.in_heading;
            self.paras.push(std::mem::take(&mut self.cur));
        } else {
            self.cur = Paragraph::default();
        }
        self.cur_len = 0;
    }
    /// An illustration becomes a paragraph of its own.
    fn image(&mut self, href: &str) {
        let href = href.trim();
        if href.is_empty() || href.starts_with("data:") || href.contains("://") {
            return;
        }
        self.flush();
        let path = resolve(self.base, href);
        // The same picture twice in a row (e.g. <svg> with an <img> fallback).
        if self.paras.last().is_some_and(|p| p.image.as_deref() == Some(path.as_str())) {
            return;
        }
        self.paras.push(Paragraph { image: Some(path), ..Default::default() });
    }
}

/// `<img>` that stands for a single character (外字) rather than an illustration.
fn is_gaiji(e: &scraper::node::Element) -> bool {
    e.attr("class").is_some_and(|c| c.to_lowercase().contains("gaiji"))
}

fn text_excluding(el: ElementRef, exclude: &[&str]) -> String {
    let mut s = String::new();
    for child in el.children() {
        match child.value() {
            Node::Text(t) => s.push_str(t),
            Node::Element(e) if !exclude.contains(&e.name()) => {
                if let Some(er) = ElementRef::wrap(child) {
                    s.push_str(&text_excluding(er, exclude));
                }
            }
            _ => {}
        }
    }
    s.replace(['\n', '\r', '\t'], "")
}

fn rt_text(el: ElementRef) -> String {
    let mut s = String::new();
    for d in el.descendants() {
        if let Node::Element(e) = d.value() {
            if e.name() == "rt" {
                if let Some(er) = ElementRef::wrap(d) {
                    s.push_str(&er.text().collect::<String>());
                }
            }
        }
    }
    s.trim().to_string()
}

fn walk(el: ElementRef, c: &mut Collector<'_>) {
    for child in el.children() {
        match child.value() {
            Node::Text(t) => c.push_text(t),
            Node::Element(e) => {
                let name = e.name();
                if name == "img" && !is_gaiji(e) {
                    if let Some(src) = e.attr("src") {
                        c.image(src);
                    }
                    continue;
                }
                if name == "svg" {
                    // Full-page illustrations are often <svg><image xlink:href=…/></svg>.
                    for d in child.descendants() {
                        if let Node::Element(img) = d.value() {
                            if img.name() == "image" {
                                // `xlink:href` is namespaced, so match the local name.
                                if let Some((_, h)) = img.attrs().find(|(k, _)| *k == "href") {
                                    c.image(h);
                                }
                            }
                        }
                    }
                    continue;
                }
                if SKIP.contains(&name) {
                    continue;
                }
                let Some(er) = ElementRef::wrap(child) else { continue };
                match name {
                    "br" => c.flush(),
                    "ruby" => {
                        let base = text_excluding(er, &["rt", "rp"]);
                        let reading = rt_text(er);
                        let start = c.cur_len;
                        c.push_text(&base);
                        let end = c.cur_len;
                        if !reading.is_empty() && end > start {
                            c.cur.ruby.push(RubySpan { start, end, reading });
                        }
                    }
                    n if BLOCKS.contains(&n) => {
                        c.flush();
                        if c.heading.is_none() && matches!(n, "h1" | "h2" | "h3") {
                            let t = text_excluding(er, &["rt", "rp"]).trim().to_string();
                            if !t.is_empty() {
                                c.heading = Some(t);
                            }
                        }
                        let h = matches!(n, "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
                        let prev = c.in_heading;
                        c.in_heading |= h;
                        walk(er, c);
                        c.flush();
                        c.in_heading = prev;
                    }
                    _ => walk(er, c),
                }
            }
            _ => {}
        }
    }
}

/// Returns the paragraphs of a content document, plus its first heading.
/// `base` is the document's directory inside the EPUB, for image paths.
pub fn extract_paragraphs(html: &str, base: &str) -> (Vec<Paragraph>, Option<String>) {
    let doc = Html::parse_document(html);
    let body_sel = scraper::Selector::parse("body").unwrap();
    let root = doc.select(&body_sel).next().unwrap_or_else(|| doc.root_element());
    let mut c = Collector { base, in_heading: false, paras: Vec::new(), cur: Paragraph::default(), cur_len: 0, heading: None };
    walk(root, &mut c);
    c.flush();
    (c.paras, c.heading)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(resolve("OEBPS", "text/a.html#x"), "OEBPS/text/a.html");
        assert_eq!(resolve("OEBPS/text", "../img/c%20d.jpg"), "OEBPS/img/c d.jpg");
        assert_eq!(resolve("", "a.html"), "a.html");
    }

    #[test]
    fn paragraphs_and_ruby() {
        let html = r#"<html><body><div><p class="x">　私、<ruby>小<rp>(</rp><rt>こ</rt><rp>)</rp></ruby><ruby>柳<rt>やなぎ</rt></ruby>は</p>
            <p>　</p><p>二行目<br/>三行目</p></div></body></html>"#;
        let (p, _) = extract_paragraphs(html, "");
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].text, "　私、小柳は");
        assert_eq!(
            p[0].ruby,
            vec![
                RubySpan { start: 3, end: 4, reading: "こ".into() },
                RubySpan { start: 4, end: 5, reading: "やなぎ".into() }
            ]
        );
        assert_eq!(p[1].text, "二行目");
        assert_eq!(p[2].text, "三行目");
    }

    #[test]
    fn illustrations() {
        let html = r#"<html><body><p>前。</p>
            <div class="p-image"><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="../image/i-001.jpg"/></svg></div>
            <p>字<img class="gaiji" src="../image/g.png"/>後。</p><p><img src="../image/i-002.jpg" alt=""/></p></body></html>"#;
        let (p, _) = extract_paragraphs(html, "OEBPS/text");
        let shape: Vec<(&str, Option<&str>)> = p.iter().map(|p| (p.text.as_str(), p.image.as_deref())).collect();
        assert_eq!(
            shape,
            vec![("前。", None), ("", Some("OEBPS/image/i-001.jpg")), ("字後。", None), ("", Some("OEBPS/image/i-002.jpg"))]
        );
    }
}
