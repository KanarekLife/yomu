//! Thin Tauri adapter over `reader_core::Core` (ADR-0002, ADR-0010).

use std::sync::Arc;

use reader_core::dict::{DictionaryInfo, ImportProgress, LookupResult};
use reader_core::library::{BookDetail, BookSummary, SentenceView};
use reader_core::llm::{LlmConfig, Translation};
use reader_core::sync::{SyncMode, SyncReport, SyncStatus};
use reader_core::{Core, Error};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_fs::{FilePath, FsExt, OpenOptions};

type Res<T> = Result<T, String>;
type CoreState<'a> = State<'a, Arc<Core>>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Run blocking core work off the async runtime's worker threads.
async fn blocking<T: Send + 'static>(
    core: &CoreState<'_>,
    f: impl FnOnce(&Core) -> Result<T, Error> + Send + 'static,
) -> Res<T> {
    let core = Arc::clone(core.inner());
    tauri::async_runtime::spawn_blocking(move || f(&core)).await.map_err(err)?.map_err(err)
}

fn read_picked(app: &AppHandle, path: FilePath) -> Res<std::fs::File> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    app.fs().open(path, opts).map_err(err)
}

/// Shows the system picker; returns None when cancelled.
fn pick(app: &AppHandle, name: &str, ext: &[&str]) -> Option<FilePath> {
    let builder = app.dialog().file().set_title(name);
    // Android/iOS pickers work on MIME types; extension filters would hide
    // files such as .epub that have no registered type on many devices.
    #[cfg(desktop)]
    let builder = builder.add_filter(name, ext);
    #[cfg(mobile)]
    let _ = ext;
    builder.blocking_pick_file()
}

#[tauri::command]
async fn list_books(core: CoreState<'_>) -> Res<Vec<BookSummary>> {
    blocking(&core, |c| c.list_books()).await
}

#[tauri::command]
async fn book_detail(core: CoreState<'_>, id: i64) -> Res<BookDetail> {
    blocking(&core, move |c| c.book_detail(id)).await
}

#[tauri::command]
async fn book_cover(core: CoreState<'_>, id: i64) -> Res<tauri::ipc::Response> {
    let cover = blocking(&core, move |c| c.cover(id)).await?;
    Ok(tauri::ipc::Response::new(cover.map(|(b, _)| b).unwrap_or_default()))
}

#[tauri::command]
async fn book_image(core: CoreState<'_>, id: i64, path: String) -> Res<tauri::ipc::Response> {
    let (bytes, mime) = blocking(&core, move |c| c.book_image(id, &path)).await?;
    // MIME type first (one line), then the image bytes.
    let mut out = Vec::with_capacity(mime.len() + 1 + bytes.len());
    out.extend_from_slice(mime.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(&bytes);
    Ok(tauri::ipc::Response::new(out))
}

#[tauri::command]
async fn delete_book(core: CoreState<'_>, id: i64) -> Res<()> {
    blocking(&core, move |c| c.delete_book(id)).await
}

/// Opens the file picker and imports the chosen EPUB.
#[tauri::command]
async fn import_book(app: AppHandle, core: CoreState<'_>) -> Res<Option<BookSummary>> {
    let app2 = app.clone();
    let Some(path) = tauri::async_runtime::spawn_blocking(move || pick(&app2, "EPUB", &["epub"])).await.map_err(err)? else {
        return Ok(None);
    };
    let mut file = read_picked(&app, path)?;
    blocking(&core, move |c| {
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut bytes)?;
        c.import_epub(bytes)
    })
    .await
    .map(Some)
}

/// Imports an EPUB from a path (desktop drag & drop).
#[tauri::command]
async fn import_book_path(core: CoreState<'_>, path: String) -> Res<BookSummary> {
    blocking(&core, move |c| c.import_epub(std::fs::read(path)?)).await
}

#[tauri::command]
async fn sentences(core: CoreState<'_>, book_id: i64, from: i64, count: i64) -> Res<Vec<SentenceView>> {
    blocking(&core, move |c| c.sentences(book_id, from, count)).await
}

#[tauri::command]
async fn set_position(core: CoreState<'_>, book_id: i64, position: i64) -> Res<()> {
    blocking(&core, move |c| c.set_position(book_id, position)).await
}

#[tauri::command]
async fn lookup(core: CoreState<'_>, text: String, offset: usize) -> Res<LookupResult> {
    blocking(&core, move |c| c.lookup(&text, offset, 24)).await
}

#[tauri::command]
async fn get_settings(core: CoreState<'_>) -> Res<serde_json::Value> {
    blocking(&core, |c| c.get_settings()).await
}

#[tauri::command]
async fn set_settings(core: CoreState<'_>, value: serde_json::Value) -> Res<()> {
    blocking(&core, move |c| c.set_settings(&value)).await
}

#[tauri::command]
async fn translate(core: CoreState<'_>, book_id: i64, idx: i64, force: bool) -> Res<Translation> {
    let core = Arc::clone(core.inner());
    core.translate(book_id, idx, force).await.map_err(err)
}

#[tauri::command]
async fn cached_translation(core: CoreState<'_>, book_id: i64, idx: i64) -> Res<Option<Translation>> {
    blocking(&core, move |c| {
        let lang = c.llm_config()?.target_language;
        c.cached_translation(book_id, idx, lang.trim())
    })
    .await
}

// ---- sync (ADR-0016) -------------------------------------------------------

static SYNCING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[tauri::command]
async fn sync_status(core: CoreState<'_>) -> Res<SyncStatus> {
    blocking(&core, |c| Ok(c.sync_status())).await
}

#[tauri::command]
async fn sync_configure(core: CoreState<'_>, mode: SyncMode, folder: String, device_name: String) -> Res<SyncStatus> {
    blocking(&core, move |c| c.sync_configure(mode, folder, device_name)).await
}

/// Desktop folder picker for folder sync (Android uses a typed path).
#[tauri::command]
async fn pick_folder(app: AppHandle) -> Res<Option<String>> {
    #[cfg(desktop)]
    {
        let picked = tauri::async_runtime::spawn_blocking(move || {
            app.dialog().file().set_title("Sync folder").blocking_pick_folder()
        })
        .await
        .map_err(err)?;
        Ok(picked.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
    }
    #[cfg(mobile)]
    {
        let _ = app;
        Ok(None)
    }
}

#[tauri::command]
async fn sync_login(
    core: CoreState<'_>,
    server_url: String,
    username: String,
    password: String,
    device_name: String,
) -> Res<SyncStatus> {
    let core = Arc::clone(core.inner());
    core.sync_login(&server_url, &username, &password, &device_name).await.map_err(err)
}

#[tauri::command]
async fn sync_logout(core: CoreState<'_>) -> Res<SyncStatus> {
    let core = Arc::clone(core.inner());
    core.sync_logout().await.map_err(err)
}

#[tauri::command]
async fn sync_change_password(core: CoreState<'_>, current: String, new: String) -> Res<()> {
    let core = Arc::clone(core.inner());
    core.sync_change_password(&current, &new).await.map_err(err)
}

/// Desktop: closing the window first lets the frontend save the reading
/// position and sync (it answers with `exit_app`). A second close, or 10 s
/// without an answer, closes anyway.
static CLOSING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg_attr(mobile, allow(unused_variables))]
fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    #[cfg(desktop)]
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        if CLOSING.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        api.prevent_close();
        let _ = window.emit("app-closing", ());
        let app = window.app_handle().clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(10));
            app.exit(0);
        });
    }
}

#[tauri::command]
fn exit_app(app: AppHandle) {
    app.exit(0);
}

/// One sync run; a second request while one is running is a no-op.
#[tauri::command]
async fn sync_now(app: AppHandle, core: CoreState<'_>) -> Res<Option<SyncReport>> {
    use std::sync::atomic::Ordering;
    if SYNCING.swap(true, Ordering::SeqCst) {
        return Ok(None);
    }
    let core = Arc::clone(core.inner());
    let r = tauri::async_runtime::spawn(async move { core.sync_now().await }).await;
    SYNCING.store(false, Ordering::SeqCst);
    let rep = r.map_err(err)?.map_err(err)?;
    if rep.books_added + rep.books_updated + rep.books_deleted > 0 {
        let _ = app.emit("library-changed", rep.books_added + rep.books_updated + rep.books_deleted);
    }
    if rep.settings_changed {
        let _ = app.emit("settings-changed", ());
    }
    Ok(Some(rep))
}

#[tauri::command]
async fn test_llm(core: CoreState<'_>, config: LlmConfig) -> Res<String> {
    let core = Arc::clone(core.inner());
    core.test_llm(&config).await.map_err(err)
}

#[tauri::command]
async fn list_models(config: LlmConfig) -> Res<Vec<String>> {
    reader_core::llm::list_models(&config).await.map_err(err)
}

#[tauri::command]
async fn list_dictionaries(core: CoreState<'_>) -> Res<Vec<DictionaryInfo>> {
    blocking(&core, |c| c.list_dictionaries()).await
}

/// Opens the file picker and imports a Yomitan dictionary zip, emitting
/// `dict-import-progress` events.
#[tauri::command]
async fn import_dictionary(app: AppHandle, core: CoreState<'_>) -> Res<Option<DictionaryInfo>> {
    let app2 = app.clone();
    let Some(path) = tauri::async_runtime::spawn_blocking(move || pick(&app2, "Yomitan dictionary", &["zip"])).await.map_err(err)? else {
        return Ok(None);
    };
    let file = read_picked(&app, path)?;
    let emitter = app.clone();
    blocking(&core, move |c| {
        c.import_dictionary(std::io::BufReader::new(file), "", |p: ImportProgress| {
            let _ = emitter.emit("dict-import-progress", p);
        })
    })
    .await
    .map(Some)
}

#[tauri::command]
fn dictionary_catalog() -> Vec<reader_core::download::CatalogEntry> {
    reader_core::download::CATALOG.to_vec()
}

/// Download a dictionary zip from `url` and install it. `source` is the
/// catalog id (or empty for a custom URL); `name` labels progress events.
#[tauri::command]
async fn install_dictionary_url(
    app: AppHandle,
    core: CoreState<'_>,
    url: String,
    source: String,
    name: String,
) -> Res<DictionaryInfo> {
    let url = url.trim().to_string();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("the URL must start with https://".into());
    }
    let tmp = core.data_dir().join(format!("download-{}.zip", std::process::id()));
    let emitter = app.clone();
    let label = name.clone();
    let res = reader_core::download::download(&url, &tmp, |done, total| {
        let _ = emitter.emit(
            "dict-import-progress",
            ImportProgress { name: label.clone(), stage: "downloading".into(), done, total },
        );
    })
    .await;
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    let emitter = app.clone();
    let tmp2 = tmp.clone();
    let out = blocking(&core, move |c| {
        let f = std::fs::File::open(&tmp2)?;
        c.import_dictionary(std::io::BufReader::new(f), &source, |mut p: ImportProgress| {
            p.name = name.clone();
            let _ = emitter.emit("dict-import-progress", p);
        })
    })
    .await;
    let _ = std::fs::remove_file(&tmp);
    out
}

#[tauri::command]
async fn set_dictionary_enabled(core: CoreState<'_>, id: i64, enabled: bool) -> Res<()> {
    blocking(&core, move |c| c.set_dictionary_enabled(id, enabled)).await
}

#[tauri::command]
async fn reorder_dictionaries(core: CoreState<'_>, ids: Vec<i64>) -> Res<()> {
    blocking(&core, move |c| c.reorder_dictionaries(&ids)).await
}

#[tauri::command]
async fn delete_dictionary(core: CoreState<'_>, id: i64) -> Res<()> {
    blocking(&core, move |c| c.delete_dictionary(id)).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let core = Arc::new(Core::open(&dir)?);
            app.manage(Arc::clone(&core));
            // Rebuild books made by an older importer (chapter detection etc.).
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if let Ok(n) = core.upgrade_books() {
                    if n > 0 {
                        let _ = handle.emit("library-changed", n);
                    }
                }
            });
            // Warm up the tokenizer (loads the embedded dictionary).
            std::thread::spawn(|| {
                let _ = reader_core::nlp::Analyzer::global();
            });
            Ok(())
        })
        .on_window_event(on_window_event)
        .invoke_handler(tauri::generate_handler![
            list_books,
            book_detail,
            book_cover,
            book_image,
            sync_status,
            pick_folder,
            sync_configure,
            sync_login,
            sync_logout,
            sync_change_password,
            exit_app,
            sync_now,
            delete_book,
            import_book,
            import_book_path,
            sentences,
            set_position,
            lookup,
            get_settings,
            set_settings,
            translate,
            cached_translation,
            test_llm,
            list_models,
            list_dictionaries,
            import_dictionary,
            dictionary_catalog,
            install_dictionary_url,
            set_dictionary_enabled,
            reorder_dictionaries,
            delete_dictionary,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
