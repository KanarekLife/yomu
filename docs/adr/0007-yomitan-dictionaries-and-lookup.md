# 0007. Built-in Yomitan-compatible dictionaries and lookup

- Status: accepted
- Date: 2026-11-12

## Context

Users want "Yomitan inside the reader": tap a word, see dictionary entries
with deinflection, frequency and pitch accent. Learners already own
Yomitan dictionaries (JMdict/Jitendex, frequency lists, pitch dictionaries).
Bundling JMdict would raise licensing (CC-BY-SA attribution) and size issues,
and it would go stale.

## Decision

- **Import Yomitan dictionary `.zip` files** (format 1/3): `term_bank_*`,
  `term_meta_bank_*` (freq and pitch), `tag_bank_*`. Rows go into SQLite with
  indexes on `expression` and `reading`. Glossaries are stored as raw JSON and
  rendered in the UI, including **structured content** (Jitendex), through a
  whitelist DOM renderer (no `innerHTML`).
- **Lookup algorithm (Yomitan-style):** from the tapped character, take
  prefixes of up to 20 chars, stopping at punctuation, longest first. Each
  prefix (plus its katakana→hiragana variant) is **deinflected** with a
  rule table (`deinflect.rs`, modelled on Yomichan's `deinflect.json`: rules
  rewrite a suffix and carry word-class flags so chains such as
  `走らされてしまった → 走らされてしまう → 走らされる → 走らせる?/走る` stay
  type-correct). Candidates are queried in batched `IN (…)` statements and
  filtered by rule compatibility. Results are ranked by match length, then
  expression-vs-reading match, then fewer deinflections, then score, then
  dictionary priority, and grouped by (expression, reading) across dictionaries.
- Dictionary priority, enable/disable and deletion are user-controlled.
- Interaction: tap or click a word to open the popup and highlight the matched
  span. It is a bottom sheet on narrow screens and a floating card on wide
  ones. Without any dictionary, the popup still shows the tokenizer's lemma,
  reading and part of speech, plus a hint to install a dictionary.

## Consequences

- Works with whatever dictionaries the user prefers, fully offline.
- First run needs a dictionary import step (documented in the README).
- Kanji dictionaries (`kanji_bank`) are not supported yet (follow-up).
