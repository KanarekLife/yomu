//! Books, sentences and reading progress (ADR-0005, ADR-0006).

use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use crate::epub::parse_epub;
use crate::nlp::{Analyzer, Token};
use crate::segment::{is_heading, split_sentences, RubySpan};

/// Bump when segmentation/chapter detection changes; re-importing a book
/// made by an older importer rebuilds it in place (keeping the position).
/// 3: illustrations are stored as steps (`sentences.image`).
pub const IMPORT_VERSION: i64 = 3;
use crate::{now, now_ms, Core, Error, Result};

#[derive(Debug, Clone, Serialize)]
pub struct BookSummary {
    pub id: i64,
    pub title: String,
    pub author: String,
    pub sentence_count: i64,
    pub position: i64,
    pub added_at: i64,
    pub last_read_at: Option<i64>,
    pub has_cover: bool,
    /// Imported by an older importer and the original file is not stored, so
    /// it can only be refreshed by importing the EPUB again.
    pub outdated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Chapter {
    pub idx: i64,
    pub title: String,
    pub start: i64,
    /// 0 = table-of-contents entry, 1 = heading found inside the text.
    pub level: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BookDetail {
    #[serde(flatten)]
    pub summary: BookSummary,
    pub chapters: Vec<Chapter>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SentenceView {
    pub idx: i64,
    pub chapter: i64,
    pub paragraph_start: bool,
    pub text: String,
    pub tokens: Vec<Token>,
    /// Illustration step: path of the image inside the EPUB (`text` is empty).
    pub image: Option<String>,
}

fn fnv1a(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}-{}", bytes.len())
}

const SUMMARY_COLS: &str = "id, title, author, sentence_count, position, added_at, last_read_at, cover IS NOT NULL,
     (import_version < 2 AND epub IS NULL)";

fn row_summary(r: &rusqlite::Row) -> rusqlite::Result<BookSummary> {
    Ok(BookSummary {
        id: r.get(0)?,
        title: r.get(1)?,
        author: r.get(2)?,
        sentence_count: r.get(3)?,
        position: r.get(4)?,
        added_at: r.get(5)?,
        last_read_at: r.get(6)?,
        has_cover: r.get(7)?,
        outdated: r.get(8)?,
    })
}

impl Core {
    /// Parse an EPUB, segment it into sentences and store it. Importing the
    /// same file twice returns the existing book.
    pub fn import_epub(&self, bytes: Vec<u8>) -> Result<BookSummary> {
        let hash = fnv1a(&bytes);
        let existing: Option<(i64, i64, i64)> = self
            .db()
            .query_row(
                "SELECT id, import_version, position FROM books WHERE file_hash = ?1",
                [&hash],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let mut keep_position = None;
        if let Some((id, version, position)) = existing {
            if version >= IMPORT_VERSION {
                return self.book_summary(id);
            }
            // Outdated import: rebuild it, keeping the reading position
            // (as a count of text sentences before it).
            let images_before: i64 = self.db().query_row(
                "SELECT COUNT(*) FROM sentences WHERE book_id = ?1 AND idx < ?2 AND image IS NOT NULL",
                params![id, position],
                |r| r.get(0),
            )?;
            keep_position = Some(position - images_before);
            // Not a user deletion: no sync tombstone.
            self.db().execute("DELETE FROM books WHERE id = ?1", [id])?;
        }
        let book = parse_epub(bytes.clone())?;
        if book.chapters.is_empty() {
            return Err(Error::Epub("the book contains no readable text".into()));
        }

        let mut db = self.db();
        let tx = db.transaction()?;
        let (cover, mime) = match book.cover {
            Some((b, m)) => (Some(b), Some(m)),
            None => (None, None),
        };
        tx.execute(
            "INSERT INTO books(title, author, language, file_hash, cover, cover_mime, added_at, import_version, position, epub, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![book.title, book.author, book.language, hash, cover, mime, now(), IMPORT_VERSION, keep_position.unwrap_or(0), bytes, now_ms()],
        )?;
        let book_id = tx.last_insert_rowid();
        let mut idx: i64 = 0;
        {
            let mut ins_ch =
                tx.prepare("INSERT INTO chapters(book_id, idx, title, start_sentence, level) VALUES (?1,?2,?3,?4,?5)")?;
            let mut ins_s = tx.prepare(
                "INSERT INTO sentences(book_id, idx, chapter_idx, paragraph_start, text, ruby, image) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            )?;
            let mut ch_idx: i64 = -1;
            for ch in &book.chapters {
                let mut toc_pending = true;
                for p in &ch.paragraphs {
                    if let Some(img) = &p.image {
                        if toc_pending {
                            ch_idx += 1;
                            ins_ch.execute(params![book_id, ch_idx, ch.title, idx, 0])?;
                            toc_pending = false;
                        }
                        ins_s.execute(params![book_id, idx, ch_idx.max(0), true, "", None::<String>, img])?;
                        idx += 1;
                        continue;
                    }
                    let heading = is_heading(p);
                    let text = p.text.trim_matches(|c: char| c.is_whitespace() || c == '\u{3000}');
                    if toc_pending {
                        ch_idx += 1;
                        ins_ch.execute(params![book_id, ch_idx, ch.title, idx, 0])?;
                        toc_pending = false;
                    }
                    if heading && text != ch.title {
                        ch_idx += 1;
                        ins_ch.execute(params![book_id, ch_idx, text, idx, 1])?;
                    }
                    for s in split_sentences(p) {
                        let ruby = if s.ruby.is_empty() { None } else { Some(serde_json::to_string(&s.ruby)?) };
                        ins_s.execute(params![book_id, idx, ch_idx, s.paragraph_start, s.text, ruby, None::<String>])?;
                        idx += 1;
                    }
                }
            }
        }
        if idx == 0 {
            return Err(Error::Epub("the book contains no readable text".into()));
        }
        if let Some(p) = keep_position {
            // Older imports had no illustration steps: the saved position counts
            // text sentences only, so find the same sentence among the new steps.
            let new: Option<i64> = tx
                .query_row(
                    "SELECT idx FROM sentences WHERE book_id = ?1 AND image IS NULL ORDER BY idx LIMIT 1 OFFSET ?2",
                    params![book_id, p],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(new) = new {
                tx.execute("UPDATE books SET position = ?1 WHERE id = ?2", params![new, book_id])?;
            }
        }
        tx.execute(
            "UPDATE books SET sentence_count = ?1, position = MIN(position, ?1 - 1) WHERE id = ?2",
            params![idx, book_id],
        )?;
        // Imported again after being deleted: the import wins over the tombstone.
        tx.execute("DELETE FROM sync_tombstones WHERE key = ?1", [&hash])?;
        tx.commit()?;
        drop(db);
        // Synced metadata/translations that were waiting for this file.
        self.apply_pending(&hash)?;
        self.book_summary(book_id)
    }

    /// Re-process books imported by an older importer whose EPUB is stored.
    /// Returns how many books were upgraded.
    pub fn upgrade_books(&self) -> Result<usize> {
        let outdated: Vec<Vec<u8>> = {
            let db = self.db();
            let mut st = db.prepare("SELECT epub FROM books WHERE import_version < ?1 AND epub IS NOT NULL")?;
            let rows = st.query_map([IMPORT_VERSION], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let n = outdated.len();
        for bytes in outdated {
            self.import_epub(bytes)?;
        }
        Ok(n)
    }

    pub fn book_summary(&self, id: i64) -> Result<BookSummary> {
        self.db()
            .query_row(&format!("SELECT {SUMMARY_COLS} FROM books WHERE id = ?1"), [id], row_summary)
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("book {id} not found")))
    }

    pub fn list_books(&self) -> Result<Vec<BookSummary>> {
        let db = self.db();
        let mut st = db.prepare(&format!(
            "SELECT {SUMMARY_COLS} FROM books ORDER BY COALESCE(last_read_at, added_at) DESC"
        ))?;
        let rows = st.query_map([], row_summary)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn book_detail(&self, id: i64) -> Result<BookDetail> {
        let summary = self.book_summary(id)?;
        let db = self.db();
        let mut st =
            db.prepare("SELECT idx, title, start_sentence, level FROM chapters WHERE book_id = ?1 ORDER BY idx")?;
        let chapters = st
            .query_map([id], |r| Ok(Chapter { idx: r.get(0)?, title: r.get(1)?, start: r.get(2)?, level: r.get(3)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(BookDetail { summary, chapters })
    }

    pub fn cover(&self, id: i64) -> Result<Option<(Vec<u8>, String)>> {
        Ok(self
            .db()
            .query_row("SELECT cover, cover_mime FROM books WHERE id = ?1", [id], |r| {
                Ok((r.get::<_, Option<Vec<u8>>>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .optional()?
            .and_then(|(b, m)| b.map(|b| (b, m.unwrap_or_else(|| "image/jpeg".into())))))
    }

    /// An illustration from the book's stored EPUB, with its MIME type.
    pub fn book_image(&self, id: i64, path: &str) -> Result<(Vec<u8>, String)> {
        let epub = {
            let mut cache = self.epub_cache.lock().unwrap_or_else(|p| p.into_inner());
            match cache.as_ref() {
                Some((cid, b)) if *cid == id => b.clone(),
                _ => {
                    let b: Option<Vec<u8>> =
                        self.db().query_row("SELECT epub FROM books WHERE id = ?1", [id], |r| r.get(0)).optional()?.flatten();
                    let b = std::sync::Arc::new(b.ok_or_else(|| Error::NotFound(format!("book {id} has no stored EPUB")))?);
                    *cache = Some((id, b.clone()));
                    b
                }
            }
        };
        let bytes = crate::epub::read_entry(&epub, path)
            .ok_or_else(|| Error::NotFound(format!("image {path} not found in the EPUB")))?;
        let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        let mime = match ext.as_str() {
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "avif" => "image/avif",
            _ => "image/jpeg",
        };
        Ok((bytes, mime.into()))
    }

    /// Delete a book (user action): other devices delete it too on their next sync.
    pub fn delete_book(&self, id: i64) -> Result<()> {
        *self.epub_cache.lock().unwrap_or_else(|p| p.into_inner()) = None;
        let db = self.db();
        let key: Option<String> =
            db.query_row("SELECT file_hash FROM books WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        db.execute("DELETE FROM books WHERE id = ?1", [id])?;
        if let Some(key) = key {
            db.execute(
                "INSERT INTO sync_tombstones(key, updated_at) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET updated_at = excluded.updated_at",
                params![key, now_ms()],
            )?;
            db.execute("DELETE FROM sync_pending WHERE key = ?1", [key])?;
        }
        Ok(())
    }

    pub fn set_position(&self, id: i64, position: i64) -> Result<()> {
        self.db().execute(
            "UPDATE books SET
                 updated_at = CASE WHEN position != MAX(0, MIN(?2, sentence_count - 1)) THEN ?4 ELSE updated_at END,
                 position = MAX(0, MIN(?2, sentence_count - 1)), last_read_at = ?3
             WHERE id = ?1",
            params![id, position, now(), now_ms()],
        )?;
        Ok(())
    }

    /// Raw sentence texts in `[from, from+count)` (used for LLM context).
    pub fn sentence_texts(&self, book_id: i64, from: i64, count: i64) -> Result<Vec<String>> {
        let db = self.db();
        let mut st = db.prepare_cached(
            "SELECT text FROM sentences WHERE book_id = ?1 AND idx >= ?2 AND idx < ?3 AND image IS NULL ORDER BY idx",
        )?;
        let rows = st
            .query_map(params![book_id, from, from + count], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Sentences in `[from, from+count)`, tokenised and with furigana.
    pub fn sentences(&self, book_id: i64, from: i64, count: i64) -> Result<Vec<SentenceView>> {
        let raw: Vec<(i64, i64, bool, String, Option<String>, Option<String>)> = {
            let db = self.db();
            let mut st = db.prepare_cached(
                "SELECT idx, chapter_idx, paragraph_start, text, ruby, image FROM sentences
                 WHERE book_id = ?1 AND idx >= ?2 AND idx < ?3 ORDER BY idx",
            )?;
            let rows = st
                .query_map(params![book_id, from.max(0), from + count], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let analyzer = Analyzer::global()?;
        raw.into_iter()
            .map(|(idx, chapter, paragraph_start, text, ruby, image)| {
                let hints: Vec<RubySpan> = ruby.and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
                let tokens = if image.is_some() { Vec::new() } else { analyzer.analyze(&text, &hints)? };
                Ok(SentenceView { idx, chapter, paragraph_start, text, tokens, image })
            })
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::test_core;
    use std::io::Write;

    pub fn make_epub() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("mimetype", o).unwrap();
            z.write_all(b"application/epub+zip").unwrap();
            z.start_file("META-INF/container.xml", o).unwrap();
            z.write_all(br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
            z.start_file("OEBPS/content.opf", o).unwrap();
            z.write_all(r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>テスト</dc:title><dc:creator>作者</dc:creator></metadata>
<manifest><item id="c1" href="t/c1.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="t/c2.xhtml" media-type="application/xhtml+xml"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest>
<spine toc="ncx"><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#.as_bytes()).unwrap();
            z.start_file("OEBPS/toc.ncx", o).unwrap();
            z.write_all(r#"<?xml version="1.0"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint id="a"><navLabel><text>第一章</text></navLabel><content src="t/c1.xhtml"/></navPoint><navPoint id="b"><navLabel><text>第二章</text></navLabel><content src="t/c2.xhtml#x"/></navPoint></navMap></ncx>"#.as_bytes()).unwrap();
            z.start_file("OEBPS/t/c1.xhtml", o).unwrap();
            z.write_all("<html><body><p>　吾輩は猫である。名前はまだ無い。</p><p>２</p><p>続き。</p></body></html>".as_bytes()).unwrap();
            z.start_file("OEBPS/t/c2.xhtml", o).unwrap();
            z.write_all("<html><body><p>「どこで生まれたか？　とんと見当がつかぬ」</p></body></html>".as_bytes()).unwrap();
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn import_and_read() {
        let (core, _d) = test_core();
        let b = core.import_epub(make_epub()).unwrap();
        assert_eq!(b.title, "テスト");
        assert_eq!(b.sentence_count, 6);
        // duplicate import returns the same book
        assert_eq!(core.import_epub(make_epub()).unwrap().id, b.id);
        let d = core.book_detail(b.id).unwrap();
        assert_eq!(d.chapters.iter().map(|c| (c.title.as_str(), c.start)).collect::<Vec<_>>(), vec![("第一章", 0), ("２", 2), ("第二章", 4)]);
        let s = core.sentences(b.id, 1, 4).unwrap();
        assert_eq!(s[0].text, "名前はまだ無い。");
        assert_eq!(s[1].text, "２");
        assert_eq!(s[3].text, "「どこで生まれたか？");
        core.set_position(b.id, 99).unwrap();
        assert_eq!(core.book_summary(b.id).unwrap().position, 5);
        // Simulate an older import: it gets rebuilt from the stored EPUB.
        core.db().execute("UPDATE books SET import_version = 1, position = 3", []).unwrap();
        assert_eq!(core.upgrade_books().unwrap(), 1);
        let b2 = core.list_books().unwrap().pop().unwrap();
        assert_eq!((b2.position, b2.sentence_count, b2.outdated), (3, 6, false));
        core.delete_book(b2.id).unwrap();
        assert!(core.list_books().unwrap().is_empty());
    }

    #[test]
    fn illustrations_are_steps() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            let mut put = |n: &str, b: &[u8]| {
                z.start_file(n, o).unwrap();
                z.write_all(b).unwrap();
            };
            put("META-INF/container.xml", br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="o/c.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#);
            put("o/c.opf", r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>絵</dc:title></metadata><manifest><item id="a" href="t/a.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="a"/></spine></package>"#.as_bytes());
            put("o/t/a.xhtml", "<html><body><p>一。二。</p><p><img src=\"../i/p.PNG\"/></p><p>三。</p></body></html>".as_bytes());
            put("o/i/p.PNG", b"\x89PNG fake");
            z.finish().unwrap();
        }
        let (core, _d) = test_core();
        let b = core.import_epub(buf.into_inner()).unwrap();
        let s = core.sentences(b.id, 0, 10).unwrap();
        let shape: Vec<(&str, Option<&str>)> = s.iter().map(|s| (s.text.as_str(), s.image.as_deref())).collect();
        assert_eq!(shape, vec![("一。", None), ("二。", None), ("", Some("o/i/p.PNG")), ("三。", None)]);
        assert_eq!(core.book_image(b.id, "o/i/p.PNG").unwrap(), (b"\x89PNG fake".to_vec(), "image/png".into()));
        // Translation context skips the picture.
        assert_eq!(core.sentence_texts(b.id, 0, 4).unwrap(), vec!["一。", "二。", "三。"]);
        // Upgrading keeps the reader on the same sentence.
        core.db().execute("UPDATE books SET import_version = 2, position = 3", []).unwrap();
        core.upgrade_books().unwrap();
        let b2 = core.list_books().unwrap().pop().unwrap();
        assert_eq!(core.sentences(b2.id, b2.position, 1).unwrap()[0].text, "三。");
    }
}
