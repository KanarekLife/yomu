# 0012. Reading options live in the reader, not in Settings

- Status: accepted (amends [0009](0009-reader-interaction-and-motion.md))
- Date: 2026-11-12

## Context

Furigana and translation are toggled constantly while reading, for example
to check a reading or to peek at the meaning of one hard sentence. Putting them
(and the text size, font and theme) in the Settings screen forces a context switch, and
small header icons were easy to miss, especially on phones.

## Decision

- The reader's bottom bar holds **thumb-reachable pills**: `あ Furigana`,
  `訳 Translate`, and `Aa` (labels collapse to glyphs on narrow screens). They
  sit between the prev/next buttons, below the progress bar.
- `Aa` opens a **quick options panel** over the text: a text-size slider with
  ± buttons, Mincho/Gothic, theme swatches, and switches for furigana,
  translation, breakdown expansion, next-sentence pre-translation and
  previous-sentence context. Changes apply live and persist immediately.
- The Settings screen keeps only setup-type configuration: appearance defaults,
  the LLM provider, and dictionaries. Its old "Reading" section was removed.
- Keyboard shortcuts stay: `F` and `T`.

## Consequences

- Everything a reader adjusts mid-book is one tap away. The reading surface
  stays clean because the panel is dismissed by tapping outside it.
