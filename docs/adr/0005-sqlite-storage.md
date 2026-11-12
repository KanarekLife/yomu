# 0005. SQLite for the library, progress, dictionaries and caches

- Status: accepted
- Date: 2026-11-12

## Context

We store books (already segmented into sentences), reading positions,
settings, translation caches, and Yomitan dictionaries that can hold several
hundred thousand entries, all of which need indexed lookups in under 10 ms on
phones.

## Decision

- One **SQLite** database (`library.sqlite3` in the platform app-data dir),
  accessed via `rusqlite` with the `bundled` feature (the same SQLite version
  everywhere, Android included).
- WAL mode. A shared connection behind a mutex serves interactive calls;
  dictionary import opens its own connection and runs in one transaction on
  a blocking thread, so the reader stays usable.
- Settings are a single JSON document owned by the UI (`settings.key = 'app'`);
  Rust reads only the `llm` sub-object.
- Imports are idempotent: a book is keyed by a content hash, and a dictionary
  is keyed by title. Dictionaries whose import was interrupted (`complete = 0`)
  are removed at startup.

## Consequences

- Single file, so backup and export are trivial.
- The API key is stored in the database in plain text. Follow-up: move secrets
  to the OS keychain (Android Keystore / libsecret) via a Tauri plugin.
