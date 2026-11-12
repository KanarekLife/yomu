# 0015. Illustrations are steps in the sentence sequence

- Status: accepted (amends [0006](0006-epub-import-and-sentence-segmentation.md))
- Date: 2026-11-12

## Context

Light novels have colour plates and black-and-white illustrations between
paragraphs, usually as `<img>` or as a full-page `<svg><image xlink:href>`.
The importer dropped everything except the cover, so readers lost the
pictures that go with the scene.

## Decision

- The EPUB parser turns each `<img>` and SVG `<image>` into a paragraph of its
  own that holds the image's path inside the EPUB. Inline 外字 images
  (`class` containing `gaiji`) stay skipped. An image repeated straight after
  itself (an SVG with an `<img>` fallback) is kept once.
- An illustration is stored as a **step** in `sentences` with empty `text` and
  the path in the new `image` column. It counts towards progress and
  positions like any sentence and shows full-width in the reader. Tapping it
  opens a full-screen view.
- Image bytes are read on demand from the stored EPUB (`book_image`), and
  `Core` caches the last book's EPUB bytes in memory. Nothing is copied
  out of the EPUB at import.
- `IMPORT_VERSION` goes to 3. At startup, books with a stored EPUB are rebuilt.
  The saved position is carried over as "the n-th text sentence", so the
  added image steps don't move the reader.
- The `Aa` panel has **Show illustrations**. When it is off, navigation
  skips image steps in the direction of travel.
- Image steps are never translated, never used as LLM context, and never looked up.

## Consequences

- Illustrations appear where the author placed them, including the cover page
  most EPUBs open with.
- Rebuilding gives the book a new ID, so cached translations of old books are
  lost on upgrade (the same as earlier importer upgrades).
- Books imported before the EPUB was stored can't be upgraded
  automatically. The Contents panel asks to import them again.
