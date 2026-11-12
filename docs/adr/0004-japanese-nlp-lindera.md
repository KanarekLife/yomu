# 0004. Japanese tokenization and furigana with lindera (IPADIC)

- Status: accepted
- Date: 2026-11-12

## Context

Furigana needs a reading for every kanji word in context, and the lookup
popup benefits from knowing word boundaries and lemmas. Dictionary-only
longest-match segmentation gives poor readings (e.g. 今日 vs 今/日, 行った as
いった or おこなった).

## Decision

- Use **lindera** (a pure-Rust MeCab-compatible Viterbi tokenizer) with the
  **IPADIC** dictionary **embedded in the binary** (`embed-ipadic`). No runtime
  download is needed, and it works offline on all platforms.
- The analyzer is a global `OnceLock`, warmed up on a background thread at
  app start.
- Sentences are tokenized **on demand** in windows of about 30 around the
  cursor, not at import time. This keeps imports fast and lets tokenizer
  improvements apply to existing books.
- Furigana alignment (`text::align_furigana`) matches kana runs of the surface
  against the reading so that okurigana is never covered (`食[た]べる`), and
  falls back to whole-word ruby.
- **Author ruby wins**: `<ruby>` annotations from the EPUB are stored per
  sentence and override tokenizer readings. Tokens that a ruby span straddles
  are merged. This handles character names and unusual readings, which are
  common in light novels.

## Consequences

- About 15 MB is added to the binary for IPADIC.
- IPADIC is dated (2007) and has no modern slang. Follow-up: allow UniDic as an
  optional download.
