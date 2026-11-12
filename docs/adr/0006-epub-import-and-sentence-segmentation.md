# 0006. EPUB import and sentence segmentation

- Status: accepted
- Date: 2026-11-12

## Context

Light novels are EPUB 2/3, often vertical (`vertical-rl`), produced by
Calibre or Sigil, with `<ruby>` furigana and dialogue in 「」. The reader shows
**one sentence at a time**, so segmentation quality defines the reading experience.

## Decision

- EPUB parsing in Rust: `zip` + `roxmltree` for container/OPF/NCX and
  `scraper` (html5ever) for content documents, which tolerates malformed
  XHTML. EPUB3 `nav` takes precedence over NCX. Spine order is followed and
  `linear="no"` is skipped.
- Block elements and `<br>` delimit paragraphs. `<rt>/<rp>` are removed from
  the text and recorded as ruby spans (char offsets).
- Chapters come from TOC entries (resolved to spine files). Without a TOC,
  each spine document becomes a chapter titled by its first heading.
- Sentence splitting (`segment::split_sentences`):
  - split after `。！？!?‼⁉` plus any trailing closers and repeated terminals;
  - inside brackets, split only if the sentence itself began with an opening
    bracket (dialogue: `「A。B」` → `「A。` / `B」`). Quotes embedded in
    narration (`「何？」と言った。`) stay whole;
  - trim indentation spaces and re-base ruby spans.
- **All offsets crossing the IPC boundary are Unicode scalar (char) indices.**
  The UI uses `Array.from(str)`, which handles kanji outside the BMP (e.g. 𠮟) correctly.
- The full segmented book is stored at import. The original file is not kept.

## Consequences

- Positions are sentence indices, and progress is `position / sentence_count`.
- Re-segmenting after an algorithm change requires re-importing the book.
  Follow-up: store the EPUB and offer "re-process".
- Images and illustrations are not shown. Follow-up: an illustration interstitial.
