# 0011. One-click dictionary setup from a curated catalog

- Status: accepted (amends [0007](0007-yomitan-dictionaries-and-lookup.md))
- Date: 2026-11-12

## Context

ADR-0007 required users to find, download and import Yomitan `.zip` files
themselves. In practice this is the hardest step for new users, and on Android
it means juggling a browser, the Downloads folder and a file picker.

## Decision

- Ship a **curated catalog** (`reader_core::download::CATALOG`) of verified,
  stable download links (GitHub *latest release* or raw-repo URLs):
  - **Recommended set (one click):** Jitendex (definitions), JPDB v2.2
    frequency (kana), and Kanjium pitch accents, about 50 MB in total.
  - Alternatives: JMdict English (with or without names) and BCCWJ frequency.
- The Rust side downloads (streaming, with progress events) into the app data
  dir, imports the archive, and deletes the temporary file. Each dictionary records its
  catalog id in `dictionaries.source`, which is added by a lightweight schema
  migration. Re-installing an entry whose title already exists only tags it.
- **Surfaces:** the install card appears where it is needed: in the lookup popup
  when no dictionary exists (the lookup re-runs automatically when the install
  finishes), on the library screen, and in Settings. A single global job
  (`app.dictJob`) shows progress everywhere and survives navigation.
- **Customisation stays available** under "More dictionaries and custom
  sources": individual catalog entries, *install from URL*, *import .zip file*,
  plus enable/disable, priority order and delete for installed dictionaries.

## Consequences

- First-run setup is one tap. It needs network access once; everything is offline afterwards.
- Catalog URLs can rot. They are centralised in one table, and failures show
  a clear error with the file-import fallback still available. Follow-up: allow
  overriding the catalog from a remote JSON.
