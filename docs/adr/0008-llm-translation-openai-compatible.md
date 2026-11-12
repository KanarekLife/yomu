# 0008. Sentence translation and breakdown through any OpenAI-compatible LLM

- Status: accepted
- Date: 2026-11-12

## Context

Users want an optional translation of the current sentence using an LLM they
configure, for example DeepSeek's `deepseek-flash`, OpenAI, OpenRouter, or a
local Ollama or LM Studio server. A bare translation is of limited value to a
learner, so the result should also explain the parts of the sentence.

## Decision

- Speak the **OpenAI Chat Completions protocol** (`POST {base}/chat/completions`,
  `GET {base}/models`), which nearly every provider implements. The user
  configures the base URL, API key (optional for local servers), model,
  target language, temperature, number of context sentences, JSON mode, extra
  request-body JSON (for vendor flags such as disabling thinking), and
  custom instructions. The UI ships presets (DeepSeek, OpenAI, OpenRouter,
  Ollama, LM Studio) and a "Test connection" button, and lists models from `/models`.
- The model is asked for a **JSON object**: `translation`, `literal`,
  `breakdown[] {text, reading, meaning, role}`, `grammar[] {pattern,
  explanation}`, and `notes`. `response_format: json_object` is sent when JSON
  mode is on. Parsing is tolerant: code fences, `<think>` blocks and leading
  prose are stripped, and plain text falls back to translation-only.
- The preceding N sentences are sent as **context only**, to resolve omitted
  subjects and tone.
- Results are **cached** per (book, sentence, target language). "Retranslate"
  forces a refresh. HTTP calls run in Rust (no CORS issues, and the key never
  touches the WebView's network stack).
- Translation is off by default and toggled per session (`T` key or button).
  When it is on, the next sentence is prefetched.

## Consequences

- Provider-agnostic, and works offline with local models.
- Costs are borne by the user's API account. Caching avoids repeat charges.
- The key is stored in SQLite (see ADR-0005 follow-up).
