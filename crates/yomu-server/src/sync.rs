//! Last-writer-wins sync of books, settings and translations.
//!
//! Every accepted write gets `rev = nextval('rev_seq')`; clients pull rows
//! with `rev > since`. A per-user advisory lock serialises sync transactions
//! of the same user so a cursor can never skip a row committed later with a
//! smaller rev.

use std::collections::HashMap;
use std::hash::Hash;

use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};

/// Max books returned per sync response.
pub const BOOK_PAGE: usize = 2000;
/// Max translations returned per sync response.
pub const TRANSLATION_PAGE: usize = 2000;
/// Rows per bulk upsert statement.
const CHUNK: usize = 2000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Book {
    pub key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub last_read_at: Option<i64>,
    #[serde(default)]
    pub added_at: i64,
    #[serde(default)]
    pub deleted: bool,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub data: serde_json::Value,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Translation {
    pub key: String,
    pub idx: i64,
    pub lang: String,
    pub text: String,
    #[serde(default)]
    pub model: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SyncRequest {
    #[serde(default)]
    pub since: i64,
    #[serde(default)]
    pub books: Vec<Book>,
    #[serde(default)]
    pub settings: Option<Settings>,
    #[serde(default)]
    pub translations: Vec<Translation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponse {
    pub cursor: i64,
    pub books: Vec<Book>,
    pub settings: Option<Settings>,
    pub translations: Vec<Translation>,
    pub files: Vec<String>,
    pub more: bool,
}

/// The LWW rule: an incoming record replaces the stored one only if it is
/// strictly newer (or nothing is stored). The SQL upserts below encode the
/// same rule as `WHERE stored.updated_at < EXCLUDED.updated_at`.
pub fn lww_accepts(stored_updated_at: Option<i64>, incoming_updated_at: i64) -> bool {
    match stored_updated_at {
        None => true,
        Some(s) => incoming_updated_at > s,
    }
}

pub trait LwwRecord {
    type Key: Eq + Hash;
    fn lww_key(&self) -> Self::Key;
    fn updated_at(&self) -> i64;
}

impl LwwRecord for Book {
    type Key = String;
    fn lww_key(&self) -> String {
        self.key.clone()
    }
    fn updated_at(&self) -> i64 {
        self.updated_at
    }
}

impl LwwRecord for Translation {
    type Key = (String, i64, String);
    fn lww_key(&self) -> Self::Key {
        (self.key.clone(), self.idx, self.lang.clone())
    }
    fn updated_at(&self) -> i64 {
        self.updated_at
    }
}

/// Collapses duplicate keys within one batch by the LWW rule (a bulk upsert
/// can't touch the same row twice). On equal `updated_at` the first one wins,
/// matching "replace only if strictly newer". Order of first appearance is kept.
pub fn dedupe_lww<R: LwwRecord>(records: Vec<R>) -> Vec<R> {
    let mut index: HashMap<R::Key, usize> = HashMap::with_capacity(records.len());
    let mut out: Vec<R> = Vec::with_capacity(records.len());
    for r in records {
        match index.get(&r.lww_key()) {
            Some(&i) => {
                if lww_accepts(Some(out[i].updated_at()), r.updated_at()) {
                    out[i] = r;
                }
            }
            None => {
                index.insert(r.lww_key(), out.len());
                out.push(r);
            }
        }
    }
    out
}

fn validate(req: &SyncRequest) -> ApiResult<()> {
    let bad_key = |k: &str| k.is_empty() || k.len() > 512;
    if req.since < 0 {
        return Err(ApiError::bad_request("since must be >= 0"));
    }
    if req.books.iter().any(|b| bad_key(&b.key)) {
        return Err(ApiError::bad_request("book key must be 1-512 bytes"));
    }
    if req
        .translations
        .iter()
        .any(|t| bad_key(&t.key) || t.lang.is_empty() || t.lang.len() > 128)
    {
        return Err(ApiError::bad_request(
            "translation key must be 1-512 bytes and lang 1-128 bytes",
        ));
    }
    Ok(())
}

/// Applies the incoming records and collects everything newer than `since`,
/// all in one transaction.
pub async fn run(
    client: &mut deadpool_postgres::Object,
    user_id: i64,
    req: SyncRequest,
) -> ApiResult<SyncResponse> {
    validate(&req)?;
    let since = req.since;
    let tx = client.transaction().await?;
    tx.execute(
        "SELECT pg_advisory_xact_lock(1, ($1::int8 % 2147483647)::int4)",
        &[&user_id],
    )
    .await?;

    apply_books(&tx, user_id, dedupe_lww(req.books)).await?;
    if let Some(s) = req.settings {
        tx.execute(
            "INSERT INTO settings (user_id, data, updated_at, rev)
             VALUES ($1, $2, $3, nextval('rev_seq'))
             ON CONFLICT (user_id) DO UPDATE
               SET data = EXCLUDED.data, updated_at = EXCLUDED.updated_at, rev = EXCLUDED.rev
               WHERE settings.updated_at < EXCLUDED.updated_at",
            &[&user_id, &s.data, &s.updated_at],
        )
        .await?;
    }
    apply_translations(&tx, user_id, dedupe_lww(req.translations)).await?;

    let books = tx
        .query(
            "SELECT rev, key, title, author, position, last_read_at, added_at, deleted, updated_at
             FROM books WHERE user_id = $1 AND rev > $2 ORDER BY rev LIMIT $3",
            &[&user_id, &since, &((BOOK_PAGE + 1) as i64)],
        )
        .await?
        .into_iter()
        .map(|r| {
            (
                r.get::<_, i64>(0),
                Book {
                    key: r.get(1),
                    title: r.get(2),
                    author: r.get(3),
                    position: r.get(4),
                    last_read_at: r.get(5),
                    added_at: r.get(6),
                    deleted: r.get(7),
                    updated_at: r.get(8),
                },
            )
        })
        .collect();
    let translations = tx
        .query(
            "SELECT rev, key, idx, lang, text, model, updated_at
             FROM translations WHERE user_id = $1 AND rev > $2 ORDER BY rev LIMIT $3",
            &[&user_id, &since, &((TRANSLATION_PAGE + 1) as i64)],
        )
        .await?
        .into_iter()
        .map(|r| {
            (
                r.get::<_, i64>(0),
                Translation {
                    key: r.get(1),
                    idx: r.get(2),
                    lang: r.get(3),
                    text: r.get(4),
                    model: r.get(5),
                    updated_at: r.get(6),
                },
            )
        })
        .collect();
    let settings = tx
        .query_opt(
            "SELECT rev, data, updated_at FROM settings WHERE user_id = $1 AND rev > $2",
            &[&user_id, &since],
        )
        .await?
        .map(|r| {
            (
                r.get::<_, i64>(0),
                Settings {
                    data: r.get(1),
                    updated_at: r.get(2),
                },
            )
        });
    let files = tx
        .query(
            "SELECT key FROM files WHERE user_id = $1 ORDER BY key",
            &[&user_id],
        )
        .await?
        .into_iter()
        .map(|r| r.get(0))
        .collect();
    tx.commit().await?;

    let page = paginate(since, books, translations, settings);
    Ok(SyncResponse {
        cursor: page.cursor,
        books: page.books,
        settings: page.settings,
        translations: page.translations,
        files,
        more: page.more,
    })
}

async fn apply_books(
    tx: &tokio_postgres::Transaction<'_>,
    user_id: i64,
    books: Vec<Book>,
) -> ApiResult<()> {
    for chunk in books.chunks(CHUNK) {
        let keys: Vec<&str> = chunk.iter().map(|b| b.key.as_str()).collect();
        let titles: Vec<&str> = chunk.iter().map(|b| b.title.as_str()).collect();
        let authors: Vec<&str> = chunk.iter().map(|b| b.author.as_str()).collect();
        let positions: Vec<i64> = chunk.iter().map(|b| b.position).collect();
        let last_read: Vec<Option<i64>> = chunk.iter().map(|b| b.last_read_at).collect();
        let added: Vec<i64> = chunk.iter().map(|b| b.added_at).collect();
        let deleted: Vec<bool> = chunk.iter().map(|b| b.deleted).collect();
        let updated: Vec<i64> = chunk.iter().map(|b| b.updated_at).collect();
        tx.execute(
            "INSERT INTO books (user_id, key, title, author, position, last_read_at, added_at, deleted, updated_at, rev)
             SELECT $1, t.key, t.title, t.author, t.position, t.last_read_at, t.added_at, t.deleted, t.updated_at, nextval('rev_seq')
             FROM UNNEST($2::text[], $3::text[], $4::text[], $5::int8[], $6::int8[], $7::int8[], $8::bool[], $9::int8[])
               AS t(key, title, author, position, last_read_at, added_at, deleted, updated_at)
             ON CONFLICT (user_id, key) DO UPDATE SET
               title = EXCLUDED.title, author = EXCLUDED.author, position = EXCLUDED.position,
               last_read_at = EXCLUDED.last_read_at, added_at = EXCLUDED.added_at,
               deleted = EXCLUDED.deleted, updated_at = EXCLUDED.updated_at, rev = EXCLUDED.rev
             WHERE books.updated_at < EXCLUDED.updated_at",
            &[&user_id, &keys, &titles, &authors, &positions, &last_read, &added, &deleted, &updated],
        )
        .await?;
    }
    Ok(())
}

async fn apply_translations(
    tx: &tokio_postgres::Transaction<'_>,
    user_id: i64,
    translations: Vec<Translation>,
) -> ApiResult<()> {
    for chunk in translations.chunks(CHUNK) {
        let keys: Vec<&str> = chunk.iter().map(|t| t.key.as_str()).collect();
        let idxs: Vec<i64> = chunk.iter().map(|t| t.idx).collect();
        let langs: Vec<&str> = chunk.iter().map(|t| t.lang.as_str()).collect();
        let texts: Vec<&str> = chunk.iter().map(|t| t.text.as_str()).collect();
        let models: Vec<&str> = chunk.iter().map(|t| t.model.as_str()).collect();
        let updated: Vec<i64> = chunk.iter().map(|t| t.updated_at).collect();
        tx.execute(
            "INSERT INTO translations (user_id, key, idx, lang, text, model, updated_at, rev)
             SELECT $1, t.key, t.idx, t.lang, t.text, t.model, t.updated_at, nextval('rev_seq')
             FROM UNNEST($2::text[], $3::int8[], $4::text[], $5::text[], $6::text[], $7::int8[])
               AS t(key, idx, lang, text, model, updated_at)
             ON CONFLICT (user_id, key, idx, lang) DO UPDATE SET
               text = EXCLUDED.text, model = EXCLUDED.model,
               updated_at = EXCLUDED.updated_at, rev = EXCLUDED.rev
             WHERE translations.updated_at < EXCLUDED.updated_at",
            &[&user_id, &keys, &idxs, &langs, &texts, &models, &updated],
        )
        .await?;
    }
    Ok(())
}

#[derive(Debug)]
pub struct Page {
    pub cursor: i64,
    pub more: bool,
    pub books: Vec<Book>,
    pub translations: Vec<Translation>,
    pub settings: Option<Settings>,
}

/// Turns rows fetched with `LIMIT page+1 ORDER BY rev` into one response
/// page. If any table overflowed, everything above the smallest truncation
/// point is dropped so that `cursor` never skips a row; `more` tells the
/// client to sync again from `cursor`.
pub fn paginate(
    since: i64,
    mut books: Vec<(i64, Book)>,
    mut translations: Vec<(i64, Translation)>,
    mut settings: Option<(i64, Settings)>,
) -> Page {
    let mut bound = i64::MAX;
    let mut more = false;
    if books.len() > BOOK_PAGE {
        books.truncate(BOOK_PAGE);
        bound = bound.min(books.last().map_or(since, |b| b.0));
        more = true;
    }
    if translations.len() > TRANSLATION_PAGE {
        translations.truncate(TRANSLATION_PAGE);
        bound = bound.min(translations.last().map_or(since, |t| t.0));
        more = true;
    }
    if more {
        books.retain(|b| b.0 <= bound);
        translations.retain(|t| t.0 <= bound);
        if settings.as_ref().is_some_and(|s| s.0 > bound) {
            settings = None;
        }
    }
    let cursor = books
        .iter()
        .map(|b| b.0)
        .chain(translations.iter().map(|t| t.0))
        .chain(settings.iter().map(|s| s.0))
        .fold(since, i64::max);
    Page {
        cursor,
        more,
        books: books.into_iter().map(|b| b.1).collect(),
        translations: translations.into_iter().map(|t| t.1).collect(),
        settings: settings.map(|s| s.1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(key: &str, pos: i64, updated_at: i64) -> Book {
        Book {
            key: key.into(),
            title: String::new(),
            author: String::new(),
            position: pos,
            last_read_at: None,
            added_at: 0,
            deleted: false,
            updated_at,
        }
    }

    fn tr(key: &str, idx: i64, lang: &str, text: &str, updated_at: i64) -> Translation {
        Translation {
            key: key.into(),
            idx,
            lang: lang.into(),
            text: text.into(),
            model: String::new(),
            updated_at,
        }
    }

    #[test]
    fn lww_rule() {
        assert!(lww_accepts(None, 0));
        assert!(lww_accepts(Some(5), 6));
        assert!(!lww_accepts(Some(5), 5), "ties keep the stored record");
        assert!(!lww_accepts(Some(5), 4));
    }

    #[test]
    fn dedupe_keeps_newest_per_key() {
        let out = dedupe_lww(vec![
            book("a", 1, 10),
            book("b", 1, 10),
            book("a", 2, 30),
            book("a", 3, 20),
            book("b", 2, 10), // tie: first wins
        ]);
        assert_eq!(out, vec![book("a", 2, 30), book("b", 1, 10)]);
    }

    #[test]
    fn dedupe_translations_by_composite_key() {
        let out = dedupe_lww(vec![
            tr("k", 1, "English", "old", 1),
            tr("k", 1, "Polish", "pl", 1),
            tr("k", 2, "English", "two", 1),
            tr("k", 1, "English", "new", 2),
        ]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].text, "new");
        assert_eq!(out[1].text, "pl");
    }

    #[test]
    fn paginate_without_overflow() {
        let p = paginate(
            5,
            vec![(7, book("a", 0, 0))],
            vec![(9, tr("a", 0, "en", "", 0))],
            Some((
                8,
                Settings {
                    data: serde_json::json!({}),
                    updated_at: 0,
                },
            )),
        );
        assert_eq!(p.cursor, 9);
        assert!(!p.more);
        assert_eq!(p.books.len(), 1);
        assert!(p.settings.is_some());

        let empty = paginate(42, vec![], vec![], None);
        assert_eq!(empty.cursor, 42);
        assert!(!empty.more);
    }

    #[test]
    fn paginate_truncates_to_smallest_bound() {
        // Translations overflow at rev 100 + TRANSLATION_PAGE - 1; books are
        // beyond that and must be held back for the next page.
        let trs: Vec<_> = (0..=TRANSLATION_PAGE as i64)
            .map(|i| (100 + i, tr("k", i, "en", "", 0)))
            .collect();
        let last_kept = 100 + TRANSLATION_PAGE as i64 - 1;
        let p = paginate(
            0,
            vec![
                (50, book("early", 0, 0)),
                (last_kept + 5, book("late", 0, 0)),
            ],
            trs,
            Some((
                last_kept + 1,
                Settings {
                    data: serde_json::json!(1),
                    updated_at: 0,
                },
            )),
        );
        assert!(p.more);
        assert_eq!(p.cursor, last_kept);
        assert_eq!(p.translations.len(), TRANSLATION_PAGE);
        assert_eq!(p.books.len(), 1);
        assert_eq!(p.books[0].key, "early");
        assert!(p.settings.is_none());
    }

    #[test]
    fn request_defaults() {
        let r: SyncRequest = serde_json::from_str(r#"{"since": 3}"#).unwrap();
        assert_eq!(r.since, 3);
        assert!(r.books.is_empty() && r.settings.is_none() && r.translations.is_empty());
        let r: SyncRequest = serde_json::from_str(
            r#"{"since":0,"books":[{"key":"k","title":"t","author":"a","position":1,"last_read_at":null,"added_at":2,"deleted":false,"updated_at":3}],"settings":null,"translations":[]}"#,
        )
        .unwrap();
        assert_eq!(r.books[0].updated_at, 3);
    }
}
