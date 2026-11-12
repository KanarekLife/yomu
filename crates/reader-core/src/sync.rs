//! Syncing between devices (ADR-0016).
//!
//! Everything that syncs is a *record* with an `updated_at` (Unix ms) and the
//! merge rule is last-writer-wins per record:
//!
//! - a book (keyed by the EPUB's fingerprint): position, last read, deleted;
//! - the settings blob (including the LLM key);
//! - a cached translation (book, sentence, language).
//!
//! Positions and sentence numbers are sent as "n-th text sentence" so that
//! importer differences such as illustration steps never shift them.
//!
//! Two transports share the same records and merge:
//!
//! - **server**: `yomu-server` (accounts, Postgres storage, EPUB files);
//! - **folder**: a directory synced by something else (Syncthing, Nextcloud…).
//!   Each device writes only its own `devices/<id>.json` (full state) and
//!   reads everybody else's, so the folder never has write conflicts; EPUBs go
//!   to `books/<key>.epub`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{now_ms, Core, Error, Result};

// ---- records -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookRec {
    pub key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author: String,
    /// Reading position as the index among text sentences (illustrations skipped).
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
pub struct SettingsRec {
    pub data: Value,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrRec {
    pub key: String,
    /// Index among text sentences.
    pub idx: i64,
    pub lang: String,
    /// The translation JSON, opaque to the transport.
    pub text: String,
    #[serde(default)]
    pub model: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub books: Vec<BookRec>,
    #[serde(default)]
    pub settings: Option<SettingsRec>,
    #[serde(default)]
    pub translations: Vec<TrRec>,
}

// ---- configuration & status ----------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncMode {
    #[default]
    Off,
    Server,
    Folder,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SyncConfig {
    #[serde(default)]
    mode: SyncMode,
    #[serde(default)]
    server_url: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    folder: String,
    #[serde(default)]
    device_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncStatus {
    pub mode: SyncMode,
    pub server_url: String,
    pub username: String,
    pub signed_in: bool,
    pub folder: String,
    pub device_name: String,
    pub device_id: String,
    /// Unix ms of the last successful sync.
    pub last_sync_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncReport {
    pub books_added: usize,
    pub books_updated: usize,
    pub books_deleted: usize,
    pub settings_changed: bool,
    pub translations: usize,
    pub files_uploaded: usize,
    pub files_downloaded: usize,
    /// Books known from other devices whose EPUB is not available yet.
    pub waiting_for_files: usize,
}

impl SyncReport {
    fn add(&mut self, o: SyncReport) {
        self.books_added += o.books_added;
        self.books_updated += o.books_updated;
        self.books_deleted += o.books_deleted;
        self.settings_changed |= o.settings_changed;
        self.translations += o.translations;
    }
}

const K_CONFIG: &str = "sync_config";
const K_CURSOR: &str = "sync_cursor";
const K_LAST_PUSH: &str = "sync_last_push";
const K_LAST_AT: &str = "sync_last_at";
const K_LAST_ERR: &str = "sync_last_error";
const K_DEVICE: &str = "device_id";

fn sync_err(e: impl std::fmt::Display) -> Error {
    Error::Sync(e.to_string())
}

// ---- local state <-> records -------------------------------------------------------

impl Core {
    fn config(&self) -> SyncConfig {
        self.meta(K_CONFIG).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save_config(&self, c: &SyncConfig) -> Result<()> {
        self.set_meta(K_CONFIG, Some(&serde_json::to_string(c)?))
    }

    /// Stable random id of this installation.
    pub fn device_id(&self) -> String {
        if let Some(id) = self.meta(K_DEVICE) {
            return id;
        }
        let mut b = [0u8; 8];
        getrandom(&mut b);
        let id: String = b.iter().map(|x| format!("{x:02x}")).collect();
        let _ = self.set_meta(K_DEVICE, Some(&id));
        id
    }

    fn text_ordinal(&self, book_id: i64, idx: i64) -> Result<i64> {
        Ok(self.db().query_row(
            "SELECT COUNT(*) FROM sentences WHERE book_id = ?1 AND idx < ?2 AND image IS NULL",
            params![book_id, idx],
            |r| r.get(0),
        )?)
    }

    fn idx_of_ordinal(&self, book_id: i64, ordinal: i64) -> Result<i64> {
        let db = self.db();
        let idx: Option<i64> = db
            .query_row(
                "SELECT idx FROM sentences WHERE book_id = ?1 AND image IS NULL ORDER BY idx LIMIT 1 OFFSET ?2",
                params![book_id, ordinal.max(0)],
                |r| r.get(0),
            )
            .optional()?;
        match idx {
            Some(i) => Ok(i),
            None => Ok(db.query_row("SELECT MAX(0, sentence_count - 1) FROM books WHERE id = ?1", [book_id], |r| r.get(0))?),
        }
    }

    /// Local records changed after `since` (Unix ms; -1 = everything).
    pub fn sync_export(&self, since: i64) -> Result<Snapshot> {
        let rows: Vec<(i64, String, String, String, i64, Option<i64>, i64, i64)> = {
            let db = self.db();
            let mut st = db.prepare(
                "SELECT id, file_hash, title, author, position, last_read_at, added_at, updated_at FROM books WHERE updated_at > ?1",
            )?;
            let rows = st
                .query_map([since], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let mut books = Vec::with_capacity(rows.len());
        for (id, key, title, author, position, last_read_at, added_at, updated_at) in rows {
            let position = self.text_ordinal(id, position)?;
            books.push(BookRec { key, title, author, position, last_read_at, added_at, deleted: false, updated_at });
        }
        {
            let db = self.db();
            let mut st = db.prepare("SELECT key, updated_at FROM sync_tombstones WHERE updated_at > ?1")?;
            let dead = st.query_map([since], |r| {
                Ok(BookRec {
                    key: r.get(0)?,
                    title: String::new(),
                    author: String::new(),
                    position: 0,
                    last_read_at: None,
                    added_at: 0,
                    deleted: true,
                    updated_at: r.get(1)?,
                })
            })?;
            for b in dead {
                books.push(b?);
            }
        }

        let settings_at: i64 = self.meta("app_updated_at").and_then(|s| s.parse().ok()).unwrap_or(0);
        let settings = if settings_at > since {
            let data = self.get_settings()?;
            (!data.is_null()).then_some(SettingsRec { data, updated_at: settings_at })
        } else {
            None
        };

        let tr_rows: Vec<(i64, String, i64, String, String, String, i64)> = {
            let db = self.db();
            let mut st = db.prepare(
                "SELECT t.book_id, b.file_hash, t.idx, t.lang, t.text, t.model, t.updated_at
                 FROM translations t JOIN books b ON b.id = t.book_id WHERE t.updated_at > ?1",
            )?;
            let rows = st
                .query_map([since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let mut translations = Vec::with_capacity(tr_rows.len());
        for (book_id, key, idx, lang, text, model, updated_at) in tr_rows {
            let idx = self.text_ordinal(book_id, idx)?;
            translations.push(TrRec { key, idx, lang, text, model, updated_at });
        }
        Ok(Snapshot { books, settings, translations })
    }

    fn local_book(&self, key: &str) -> Result<Option<(i64, i64)>> {
        Ok(self
            .db()
            .query_row("SELECT id, updated_at FROM books WHERE file_hash = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?)
    }

    fn set_book_from(&self, id: i64, rec: &BookRec) -> Result<()> {
        let idx = self.idx_of_ordinal(id, rec.position)?;
        self.db().execute(
            "UPDATE books SET position = ?2, last_read_at = COALESCE(?3, last_read_at), updated_at = ?4 WHERE id = ?1",
            params![id, idx, rec.last_read_at, rec.updated_at],
        )?;
        Ok(())
    }

    fn put_translation(&self, book_id: i64, t: &TrRec) -> Result<bool> {
        let idx = self.idx_of_ordinal(book_id, t.idx)?;
        let n = self.db().execute(
            "INSERT INTO translations(book_id, idx, lang, text, model, updated_at) VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(book_id, idx, lang) DO UPDATE SET text = excluded.text, model = excluded.model, updated_at = excluded.updated_at
             WHERE excluded.updated_at > translations.updated_at",
            params![book_id, idx, t.lang, t.text, t.model, t.updated_at],
        )?;
        Ok(n > 0)
    }

    /// Merge records from another device (last-writer-wins).
    pub fn sync_apply(&self, snap: &Snapshot) -> Result<SyncReport> {
        let mut rep = SyncReport::default();
        for rec in &snap.books {
            let local = self.local_book(&rec.key)?;
            let tomb: Option<i64> = self
                .db()
                .query_row("SELECT updated_at FROM sync_tombstones WHERE key = ?1", [&rec.key], |r| r.get(0))
                .optional()?;
            if rec.deleted {
                if tomb.is_some_and(|t| t >= rec.updated_at) {
                    continue;
                }
                if let Some((id, at)) = local {
                    if rec.updated_at <= at {
                        continue; // read/imported here after it was deleted elsewhere
                    }
                    self.db().execute("DELETE FROM books WHERE id = ?1", [id])?;
                    rep.books_deleted += 1;
                }
                let db = self.db();
                db.execute(
                    "INSERT INTO sync_tombstones(key, updated_at) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET updated_at = excluded.updated_at",
                    params![rec.key, rec.updated_at],
                )?;
                db.execute("DELETE FROM sync_pending WHERE key = ?1", [&rec.key])?;
                continue;
            }
            match local {
                Some((id, at)) => {
                    if rec.updated_at > at {
                        self.set_book_from(id, rec)?;
                        rep.books_updated += 1;
                    }
                }
                None => {
                    if tomb.is_some_and(|t| t >= rec.updated_at) {
                        continue; // deleted here after that change
                    }
                    let db = self.db();
                    let old: Option<String> = db
                        .query_row("SELECT record FROM sync_pending WHERE key = ?1", [&rec.key], |r| r.get(0))
                        .optional()?;
                    let older = old
                        .and_then(|s| serde_json::from_str::<BookRec>(&s).ok())
                        .map_or(true, |o| o.updated_at < rec.updated_at);
                    if older {
                        db.execute(
                            "INSERT INTO sync_pending(key, record) VALUES (?1, ?2)
                             ON CONFLICT(key) DO UPDATE SET record = excluded.record",
                            params![rec.key, serde_json::to_string(rec)?],
                        )?;
                    }
                }
            }
        }

        if let Some(s) = &snap.settings {
            let local_at: i64 = self.meta("app_updated_at").and_then(|v| v.parse().ok()).unwrap_or(0);
            if s.updated_at > local_at && !s.data.is_null() {
                self.store_settings(&s.data, s.updated_at)?;
                rep.settings_changed = true;
            }
        }

        for t in &snap.translations {
            if let Some((id, _)) = self.local_book(&t.key)? {
                if self.put_translation(id, t)? {
                    rep.translations += 1;
                }
            } else {
                // Keep it with the pending book until its EPUB arrives.
                let db = self.db();
                let cur: Option<String> = db
                    .query_row("SELECT translations FROM sync_pending WHERE key = ?1", [&t.key], |r| r.get(0))
                    .optional()?;
                if let Some(cur) = cur {
                    let mut list: Vec<TrRec> = serde_json::from_str(&cur).unwrap_or_default();
                    list.retain(|x| !(x.idx == t.idx && x.lang == t.lang));
                    list.push(t.clone());
                    db.execute(
                        "UPDATE sync_pending SET translations = ?2 WHERE key = ?1",
                        params![t.key, serde_json::to_string(&list)?],
                    )?;
                }
            }
        }
        Ok(rep)
    }

    /// Keys of books other devices have that are waiting for their EPUB here.
    pub fn sync_pending_keys(&self) -> Result<Vec<String>> {
        let db = self.db();
        let mut st = db.prepare("SELECT key FROM sync_pending")?;
        let keys = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(keys)
    }

    /// After a book is imported: take over the synced position and translations
    /// that were waiting for it.
    pub(crate) fn apply_pending(&self, key: &str) -> Result<()> {
        let row: Option<(String, String)> = self
            .db()
            .query_row("SELECT record, translations FROM sync_pending WHERE key = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((rec, trs)) = row else { return Ok(()) };
        self.db().execute("DELETE FROM sync_pending WHERE key = ?1", [key])?;
        let Some((id, _)) = self.local_book(key)? else { return Ok(()) };
        if let Ok(rec) = serde_json::from_str::<BookRec>(&rec) {
            self.set_book_from(id, &rec)?;
            if rec.added_at > 0 {
                self.db().execute("UPDATE books SET added_at = ?2 WHERE id = ?1", params![id, rec.added_at])?;
            }
        }
        for t in serde_json::from_str::<Vec<TrRec>>(&trs).unwrap_or_default() {
            self.put_translation(id, &t)?;
        }
        Ok(())
    }

    fn stored_epub(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db()
            .query_row("SELECT epub FROM books WHERE file_hash = ?1", [key], |r| r.get::<_, Option<Vec<u8>>>(0))
            .optional()?
            .flatten())
    }

    fn local_keys_with_epub(&self) -> Result<Vec<String>> {
        let db = self.db();
        let mut st = db.prepare("SELECT file_hash FROM books WHERE epub IS NOT NULL")?;
        let keys = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(keys)
    }
}

// ---- public API: configuration -------------------------------------------------------

impl Core {
    pub fn sync_status(&self) -> SyncStatus {
        let c = self.config();
        SyncStatus {
            mode: c.mode,
            signed_in: c.token.is_some(),
            server_url: c.server_url,
            username: c.username,
            folder: c.folder,
            device_name: c.device_name,
            device_id: self.device_id(),
            last_sync_at: self.meta(K_LAST_AT).and_then(|s| s.parse().ok()),
            last_error: self.meta(K_LAST_ERR),
        }
    }

    /// Choose the sync mode and folder. Switching modes starts the new
    /// transport from scratch (everything is pushed once).
    pub fn sync_configure(&self, mode: SyncMode, folder: String, device_name: String) -> Result<SyncStatus> {
        let mut c = self.config();
        if c.mode != mode || c.folder != folder {
            self.set_meta(K_CURSOR, None)?;
            self.set_meta(K_LAST_PUSH, None)?;
        }
        if mode == SyncMode::Folder && !folder.trim().is_empty() {
            let dir = PathBuf::from(folder.trim());
            std::fs::create_dir_all(dir.join("devices"))
                .map_err(|e| sync_err(format!("cannot use folder {}: {e}", dir.display())))?;
        }
        c.mode = mode;
        c.folder = folder.trim().to_string();
        c.device_name = device_name.trim().to_string();
        self.save_config(&c)?;
        Ok(self.sync_status())
    }

    pub async fn sync_login(&self, server_url: &str, username: &str, password: &str, device_name: &str) -> Result<SyncStatus> {
        let url = server_url.trim().trim_end_matches('/').to_string();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(sync_err("the server address must start with https://"));
        }
        #[derive(Deserialize)]
        struct Login {
            token: String,
            username: String,
        }
        let resp = client()?
            .post(format!("{url}/api/v1/login"))
            .json(&serde_json::json!({ "username": username.trim(), "password": password, "device_name": device_name.trim() }))
            .send()
            .await
            .map_err(|e| sync_err(format!("cannot reach the server: {e}")))?;
        let login: Login = check(resp).await?.json().await.map_err(sync_err)?;
        let mut c = self.config();
        if c.server_url != url || c.username != login.username {
            self.set_meta(K_CURSOR, None)?;
            self.set_meta(K_LAST_PUSH, None)?;
        }
        c.mode = SyncMode::Server;
        c.server_url = url;
        c.username = login.username;
        c.token = Some(login.token);
        c.device_name = device_name.trim().to_string();
        self.save_config(&c)?;
        self.set_meta(K_LAST_ERR, None)?;
        Ok(self.sync_status())
    }

    pub async fn sync_logout(&self) -> Result<SyncStatus> {
        let mut c = self.config();
        if let Some(t) = c.token.take() {
            let _ = client()?.post(format!("{}/api/v1/logout", c.server_url)).bearer_auth(t).send().await;
        }
        self.save_config(&c)?;
        self.set_meta(K_CURSOR, None)?;
        self.set_meta(K_LAST_PUSH, None)?;
        Ok(self.sync_status())
    }

    /// Run one sync with the configured transport.
    pub async fn sync_now(&self) -> Result<SyncReport> {
        let c = self.config();
        let r = match c.mode {
            SyncMode::Off => return Ok(SyncReport::default()),
            SyncMode::Server => self.sync_server(&c).await,
            SyncMode::Folder => self.sync_folder(&c),
        };
        match &r {
            Ok(_) => {
                self.set_meta(K_LAST_AT, Some(&now_ms().to_string()))?;
                self.set_meta(K_LAST_ERR, None)?;
            }
            Err(e) => self.set_meta(K_LAST_ERR, Some(&e.to_string()))?,
        }
        r
    }
}

// ---- server transport ------------------------------------------------------------------

fn client() -> Result<reqwest::Client> {
    crate::http_client()
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(sync_err)
}

async fn check(resp: reqwest::Response) -> Result<reqwest::Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let msg = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| body.chars().take(200).collect());
    Err(match status.as_u16() {
        401 => sync_err(if msg.is_empty() { "signed out: sign in again".into() } else { msg }),
        _ => sync_err(format!("server error {status}: {msg}")),
    })
}

#[derive(Deserialize)]
struct SyncResponse {
    cursor: i64,
    #[serde(default)]
    books: Vec<BookRec>,
    #[serde(default)]
    settings: Option<SettingsRec>,
    #[serde(default)]
    translations: Vec<TrRec>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    more: bool,
}

impl Core {
    async fn sync_server(&self, c: &SyncConfig) -> Result<SyncReport> {
        let token = c.token.clone().ok_or_else(|| sync_err("not signed in"))?;
        let base = c.server_url.clone();
        let http = client()?;
        let started = now_ms();
        let since_push: i64 = self.meta(K_LAST_PUSH).and_then(|s| s.parse().ok()).unwrap_or(-1);
        let mut cursor: i64 = self.meta(K_CURSOR).and_then(|s| s.parse().ok()).unwrap_or(0);

        let mut outgoing = Some(self.sync_export(since_push)?);
        let mut rep = SyncReport::default();
        let mut files: Vec<String>;
        loop {
            let out = outgoing.take().unwrap_or_default();
            let body = serde_json::json!({
                "since": cursor,
                "books": out.books,
                "settings": out.settings,
                "translations": out.translations,
            });
            let resp = http
                .post(format!("{base}/api/v1/sync"))
                .bearer_auth(&token)
                .json(&body)
                .send()
                .await
                .map_err(|e| sync_err(format!("cannot reach the server: {e}")))?;
            let r: SyncResponse = check(resp).await?.json().await.map_err(sync_err)?;
            rep.add(self.sync_apply(&Snapshot { books: r.books, settings: r.settings, translations: r.translations })?);
            cursor = r.cursor;
            files = r.files;
            self.set_meta(K_CURSOR, Some(&cursor.to_string()))?;
            if !r.more {
                break;
            }
        }
        self.set_meta(K_LAST_PUSH, Some(&started.to_string()))?;

        // EPUBs: upload ours that the server lacks, fetch the ones we wait for.
        let on_server: HashSet<&str> = files.iter().map(String::as_str).collect();
        for key in self.local_keys_with_epub()? {
            if on_server.contains(key.as_str()) {
                continue;
            }
            let Some(bytes) = self.stored_epub(&key)? else { continue };
            let resp = http
                .put(format!("{base}/api/v1/files/{key}"))
                .bearer_auth(&token)
                .header("content-type", "application/epub+zip")
                .body(bytes)
                .send()
                .await
                .map_err(|e| sync_err(format!("upload failed: {e}")))?;
            check(resp).await?;
            rep.files_uploaded += 1;
        }
        for key in self.sync_pending_keys()? {
            if !on_server.contains(key.as_str()) {
                rep.waiting_for_files += 1;
                continue;
            }
            let resp = http
                .get(format!("{base}/api/v1/files/{key}"))
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| sync_err(format!("download failed: {e}")))?;
            let bytes = check(resp).await?.bytes().await.map_err(sync_err)?;
            self.import_epub(bytes.to_vec())?;
            rep.files_downloaded += 1;
            rep.books_added += 1;
        }
        Ok(rep)
    }
}

// ---- folder transport (Syncthing etc.) -----------------------------------------------------

#[derive(Serialize, Deserialize)]
struct DeviceFile {
    format: u32,
    device_id: String,
    #[serde(default)]
    device_name: String,
    written_at: i64,
    #[serde(flatten)]
    state: Snapshot,
}

/// Write via a temporary file + rename so a syncing tool never sees half a file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_file_name(format!(
        ".{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("yomu")
    ));
    std::fs::write(&tmp, bytes).map_err(|e| sync_err(format!("cannot write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|e| sync_err(format!("cannot write {}: {e}", path.display())))?;
    Ok(())
}

impl Core {
    fn sync_folder(&self, c: &SyncConfig) -> Result<SyncReport> {
        if c.folder.is_empty() {
            return Err(sync_err("choose a sync folder first"));
        }
        let root = PathBuf::from(&c.folder);
        let devices = root.join("devices");
        let books = root.join("books");
        for d in [&devices, &books] {
            std::fs::create_dir_all(d).map_err(|e| sync_err(format!("cannot use folder {}: {e}", d.display())))?;
        }
        let me = self.device_id();
        let mut rep = SyncReport::default();

        // 1. Everyone else's state.
        let entries = std::fs::read_dir(&devices).map_err(|e| sync_err(format!("cannot read {}: {e}", devices.display())))?;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            // Skip our own file, temporary files and Syncthing's conflict copies.
            if name.starts_with('.') || !name.ends_with(".json") || name == format!("{me}.json") || name.contains(".sync-conflict-") {
                continue;
            }
            let Ok(bytes) = std::fs::read(e.path()) else { continue };
            let Ok(f) = serde_json::from_slice::<DeviceFile>(&bytes) else { continue };
            if f.device_id == me {
                continue;
            }
            rep.add(self.sync_apply(&f.state)?);
        }

        // 2. Books we were waiting for whose EPUB has arrived.
        for key in self.sync_pending_keys()? {
            let p = books.join(format!("{key}.epub"));
            match std::fs::read(&p) {
                Ok(bytes) => {
                    self.import_epub(bytes)?;
                    rep.files_downloaded += 1;
                    rep.books_added += 1;
                }
                Err(_) => rep.waiting_for_files += 1,
            }
        }

        // 3. Our EPUBs, for the others.
        for key in self.local_keys_with_epub()? {
            let p = books.join(format!("{key}.epub"));
            if p.exists() {
                continue;
            }
            if let Some(bytes) = self.stored_epub(&key)? {
                write_atomic(&p, &bytes)?;
                rep.files_uploaded += 1;
            }
        }

        // 4. Our full state.
        let file = DeviceFile {
            format: 1,
            device_id: me.clone(),
            device_name: c.device_name.clone(),
            written_at: now_ms(),
            state: self.sync_export(-1)?,
        };
        write_atomic(&devices.join(format!("{me}.json")), &serde_json::to_vec(&file)?)?;
        Ok(rep)
    }
}

fn getrandom(buf: &mut [u8]) {
    // No extra dependency: mix time, the process id and an address. Only used
    // for a device id, which just has to differ between installations.
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.write_u32(std::process::id());
    let v = h.finish().to_le_bytes();
    for (i, b) in buf.iter_mut().enumerate() {
        *b = v[i % 8];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_core;

    fn epub() -> Vec<u8> {
        crate::library::tests::make_epub()
    }

    #[test]
    fn folder_sync_between_two_devices() {
        let (a, _da) = test_core();
        let (b, _db) = test_core();
        // Distinct device ids (both test cores start empty).
        a.set_meta(K_DEVICE, Some("aaaa")).unwrap();
        b.set_meta(K_DEVICE, Some("bbbb")).unwrap();
        let dir = tempdir("folder");
        let folder = dir.to_string_lossy().into_owned();
        a.sync_configure(SyncMode::Folder, folder.clone(), "A".into()).unwrap();
        b.sync_configure(SyncMode::Folder, folder, "B".into()).unwrap();

        // A imports, reads to text sentence 4, translates sentence 1, sets the theme.
        let book = a.import_epub(epub()).unwrap();
        a.set_position(book.id, 4).unwrap();
        a.db()
            .execute(
                "INSERT INTO translations(book_id, idx, lang, text, model, updated_at) VALUES (?1, 1, 'English', '{\"t\":1}', 'm', 5)",
                [book.id],
            )
            .unwrap();
        a.set_settings(&serde_json::json!({ "theme": "dark" })).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(a.sync_now()).unwrap();

        // B picks everything up, including the EPUB.
        let r = rt.block_on(b.sync_now()).unwrap();
        assert_eq!((r.books_added, r.files_downloaded, r.settings_changed), (1, 1, true));
        let bb = b.list_books().unwrap().pop().unwrap();
        assert_eq!(bb.position, 4);
        assert_eq!(b.get_settings().unwrap()["theme"], "dark");
        let n: i64 = b.db().query_row("SELECT COUNT(*) FROM translations", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);

        // B reads further; A follows. Then A deletes; B deletes too.
        std::thread::sleep(std::time::Duration::from_millis(5));
        b.set_position(bb.id, 2).unwrap();
        rt.block_on(b.sync_now()).unwrap();
        rt.block_on(a.sync_now()).unwrap();
        assert_eq!(a.list_books().unwrap()[0].position, 2);
        std::thread::sleep(std::time::Duration::from_millis(5));
        a.delete_book(book.id).unwrap();
        rt.block_on(a.sync_now()).unwrap();
        let r = rt.block_on(b.sync_now()).unwrap();
        assert_eq!(r.books_deleted, 1);
        assert!(b.list_books().unwrap().is_empty());
        // ...and a stale sync from B doesn't resurrect it on A.
        rt.block_on(a.sync_now()).unwrap();
        assert!(a.list_books().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn futures_are_send() {
        // Tauri runs commands on a multi-threaded runtime.
        fn send<T: Send>(_: &T) {}
        let (a, _d) = test_core();
        send(&a.sync_now());
        send(&a.sync_login("https://x", "u", "p", "d"));
        send(&a.sync_logout());
    }

    /// Against a running yomu-server: `YOMU_TEST_SERVER=http://host:port|user|password`.
    #[test]
    fn server_sync_between_two_devices() {
        let Ok(spec) = std::env::var("YOMU_TEST_SERVER") else { return };
        let mut it = spec.split('|');
        let (url, user, pw) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let (a, _da) = test_core();
        let (b, _db) = test_core();
        rt.block_on(a.sync_login(url, user, pw, "A")).unwrap();
        rt.block_on(b.sync_login(url, user, pw, "B")).unwrap();

        let book = a.import_epub(epub()).unwrap();
        a.set_position(book.id, 3).unwrap();
        a.db()
            .execute("INSERT INTO translations(book_id, idx, lang, text, model, updated_at) VALUES (?1, 2, 'English', '{}', 'm', 7)", [book.id])
            .unwrap();
        a.set_settings(&serde_json::json!({ "theme": "sepia", "llm": { "api_key": "sk-test" } })).unwrap();
        let r = rt.block_on(a.sync_now()).unwrap();
        assert_eq!(r.files_uploaded, 1);

        let r = rt.block_on(b.sync_now()).unwrap();
        assert_eq!((r.books_added, r.files_downloaded, r.settings_changed), (1, 1, true));
        let bb = b.list_books().unwrap().pop().unwrap();
        assert_eq!(bb.position, 3);
        assert_eq!(b.get_settings().unwrap()["llm"]["api_key"], "sk-test");
        let n: i64 = b.db().query_row("SELECT COUNT(*) FROM translations", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);

        std::thread::sleep(std::time::Duration::from_millis(5));
        b.set_position(bb.id, 5).unwrap();
        rt.block_on(b.sync_now()).unwrap();
        rt.block_on(a.sync_now()).unwrap();
        assert_eq!(a.book_summary(book.id).unwrap().position, 5);

        std::thread::sleep(std::time::Duration::from_millis(5));
        b.delete_book(bb.id).unwrap();
        rt.block_on(b.sync_now()).unwrap();
        let r = rt.block_on(a.sync_now()).unwrap();
        assert_eq!(r.books_deleted, 1);
        rt.block_on(a.sync_logout()).unwrap();
        assert!(!a.sync_status().signed_in);
    }

    #[test]
    fn last_writer_wins() {
        let (a, _d) = test_core();
        let book = a.import_epub(epub()).unwrap();
        let key = a.sync_export(-1).unwrap().books[0].key.clone();
        let at = a.local_book(&key).unwrap().unwrap().1;
        let rec = |pos, t| BookRec { key: key.clone(), title: String::new(), author: String::new(), position: pos, last_read_at: None, added_at: 0, deleted: false, updated_at: t };
        a.sync_apply(&Snapshot { books: vec![rec(3, at - 1)], ..Default::default() }).unwrap();
        assert_eq!(a.book_summary(book.id).unwrap().position, 0, "older change ignored");
        a.sync_apply(&Snapshot { books: vec![rec(3, at + 1)], ..Default::default() }).unwrap();
        assert_eq!(a.book_summary(book.id).unwrap().position, 3, "newer change applied");
    }

    fn tempdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("yomu-sync-test-{name}-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
