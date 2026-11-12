//! Platform-independent core of the Japanese light-novel reader.
//!
//! The Tauri shell (`src-tauri`) is a thin adapter over [`Core`]; everything
//! that can be unit-tested lives here (ADR-0002).

pub mod deinflect;
pub mod dict;
pub mod download;
pub mod epub;
pub mod library;
pub mod llm;
pub mod nlp;
pub mod segment;
pub mod text;

/// HTTP client builder shared by the LLM client and dictionary downloads.
///
/// Desktop: the OS certificate verifier (respects system/corporate CAs).
/// Android: the platform verifier needs JNI initialisation from Kotlin that
/// Tauri doesn't do, and every HTTPS request failed, so use Mozilla's roots.
pub(crate) fn http_client() -> reqwest::ClientBuilder {
    static PROVIDER: std::sync::Once = std::sync::Once::new();
    PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    #[cfg(target_os = "android")]
    {
        let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let mut tls = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        reqwest::Client::builder().tls_backend_preconfigured(tls)
    }
    #[cfg(not(target_os = "android"))]
    reqwest::Client::builder()
}

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid EPUB: {0}")]
    Epub(String),
    #[error("invalid dictionary: {0}")]
    Dict(String),
    #[error("tokenizer error: {0}")]
    Nlp(String),
    #[error("translation failed: {0}")]
    Llm(String),
    #[error("{0}")]
    NotFound(String),
}

impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS books (
    id             INTEGER PRIMARY KEY,
    title          TEXT NOT NULL,
    author         TEXT NOT NULL DEFAULT '',
    language       TEXT NOT NULL DEFAULT '',
    file_hash      TEXT NOT NULL UNIQUE,
    cover          BLOB,
    cover_mime     TEXT,
    sentence_count INTEGER NOT NULL DEFAULT 0,
    position       INTEGER NOT NULL DEFAULT 0,
    added_at       INTEGER NOT NULL,
    last_read_at   INTEGER
);
CREATE TABLE IF NOT EXISTS chapters (
    book_id        INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    idx            INTEGER NOT NULL,
    title          TEXT NOT NULL,
    start_sentence INTEGER NOT NULL,
    PRIMARY KEY (book_id, idx)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS sentences (
    book_id         INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    idx             INTEGER NOT NULL,
    chapter_idx     INTEGER NOT NULL,
    paragraph_start INTEGER NOT NULL,
    text            TEXT NOT NULL,
    ruby            TEXT,
    image           TEXT,
    PRIMARY KEY (book_id, idx)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS translations (
    book_id  INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    idx      INTEGER NOT NULL,
    lang     TEXT NOT NULL,
    text     TEXT NOT NULL,
    model    TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (book_id, idx, lang)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS dictionaries (
    id          INTEGER PRIMARY KEY,
    title       TEXT NOT NULL,
    revision    TEXT NOT NULL DEFAULT '',
    enabled     INTEGER NOT NULL DEFAULT 1,
    priority    INTEGER NOT NULL DEFAULT 0,
    term_count  INTEGER NOT NULL DEFAULT 0,
    meta_count  INTEGER NOT NULL DEFAULT 0,
    imported_at INTEGER NOT NULL,
    complete    INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS terms (
    id         INTEGER PRIMARY KEY,
    dict_id    INTEGER NOT NULL REFERENCES dictionaries(id) ON DELETE CASCADE,
    expression TEXT NOT NULL,
    reading    TEXT NOT NULL,
    def_tags   TEXT NOT NULL DEFAULT '',
    rules      TEXT NOT NULL DEFAULT '',
    score      INTEGER NOT NULL DEFAULT 0,
    glossary   TEXT NOT NULL,
    sequence   INTEGER,
    term_tags  TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS terms_expression ON terms(expression);
CREATE INDEX IF NOT EXISTS terms_reading ON terms(reading);
CREATE INDEX IF NOT EXISTS terms_dict ON terms(dict_id);
CREATE TABLE IF NOT EXISTS term_meta (
    dict_id    INTEGER NOT NULL REFERENCES dictionaries(id) ON DELETE CASCADE,
    expression TEXT NOT NULL,
    mode       TEXT NOT NULL,
    data       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS term_meta_expression ON term_meta(expression);
CREATE INDEX IF NOT EXISTS term_meta_dict ON term_meta(dict_id);
CREATE TABLE IF NOT EXISTS tags (
    dict_id  INTEGER NOT NULL REFERENCES dictionaries(id) ON DELETE CASCADE,
    name     TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT '',
    notes    TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (dict_id, name)
) WITHOUT ROWID;
"#;

pub(crate) fn open_connection(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON; PRAGMA temp_store = MEMORY;",
    )?;
    Ok(conn)
}

/// Application core: owns the database and exposes every operation the UI
/// needs. Thread-safe; long-running imports open their own connection.
pub struct Core {
    db_path: PathBuf,
    conn: Mutex<Connection>,
    /// Last EPUB read for illustrations, so paging through them is cheap.
    epub_cache: Mutex<Option<(i64, std::sync::Arc<Vec<u8>>)>>,
}

impl Core {
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let db_path = data_dir.join("library.sqlite3");
        let conn = open_connection(&db_path)?;
        conn.execute_batch(SCHEMA)?;
        // v0.2: remember which catalog entry a dictionary came from.
        let has_source: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('dictionaries') WHERE name = 'source'")?
            .exists([])?;
        if !has_source {
            conn.execute_batch("ALTER TABLE dictionaries ADD COLUMN source TEXT NOT NULL DEFAULT ''")?;
        }
        // v0.3: nested chapters (in-text headings) + importer version per book.
        let has_level: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('chapters') WHERE name = 'level'")?
            .exists([])?;
        let has_epub: bool =
            conn.prepare("SELECT 1 FROM pragma_table_info('books') WHERE name = 'epub'")?.exists([])?;
        if !has_epub {
            conn.execute_batch("ALTER TABLE books ADD COLUMN epub BLOB")?;
        }
        if !has_level {
            conn.execute_batch(
                "ALTER TABLE chapters ADD COLUMN level INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE books ADD COLUMN import_version INTEGER NOT NULL DEFAULT 1;",
            )?;
        }
        // v0.1.3: illustrations are steps of their own (path inside the EPUB).
        let has_image: bool =
            conn.prepare("SELECT 1 FROM pragma_table_info('sentences') WHERE name = 'image'")?.exists([])?;
        if !has_image {
            conn.execute_batch("ALTER TABLE sentences ADD COLUMN image TEXT")?;
        }
        // Remove dictionaries whose import was interrupted.
        conn.execute("DELETE FROM dictionaries WHERE complete = 0", [])?;
        Ok(Core { db_path, conn: Mutex::new(conn), epub_cache: Mutex::new(None) })
    }

    pub(crate) fn db(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Directory holding the database (also used for temporary downloads).
    pub fn data_dir(&self) -> &Path {
        self.db_path.parent().unwrap_or(Path::new("."))
    }

    pub(crate) fn new_connection(&self) -> Result<Connection> {
        open_connection(&self.db_path)
    }

    /// Opaque JSON settings blob owned by the UI.
    pub fn get_settings(&self) -> Result<serde_json::Value> {
        let db = self.db();
        let v: Option<String> = db
            .query_row("SELECT value FROM settings WHERE key = 'app'", [], |r| r.get(0))
            .ok();
        Ok(match v {
            Some(s) => serde_json::from_str(&s).unwrap_or(serde_json::Value::Null),
            None => serde_json::Value::Null,
        })
    }

    pub fn set_settings(&self, value: &serde_json::Value) -> Result<()> {
        self.db().execute(
            "INSERT INTO settings(key, value) VALUES('app', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [serde_json::to_string(value)?],
        )?;
        Ok(())
    }
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) fn test_core() -> (Core, tempdir::TempDir) {
    let dir = tempdir::TempDir::new();
    let core = Core::open(dir.path()).unwrap();
    (core, dir)
}

#[cfg(test)]
pub(crate) mod tempdir {
    use std::path::{Path, PathBuf};
    pub struct TempDir(PathBuf);
    impl TempDir {
        pub fn new() -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let p = std::env::temp_dir().join(format!(
                "reader-core-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
