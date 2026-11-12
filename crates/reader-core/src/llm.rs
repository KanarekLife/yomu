//! Sentence translation + breakdown through any OpenAI-compatible
//! `/chat/completions` endpoint (ADR-0008).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{Core, Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    /// e.g. `https://api.deepseek.com`, `https://api.openai.com/v1`,
    /// `http://localhost:11434/v1` (Ollama), `https://openrouter.ai/api/v1`.
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub target_language: String,
    pub temperature: f64,
    /// How many preceding sentences are sent as context.
    pub context_sentences: i64,
    /// Ask for `response_format: json_object` (not every server supports it).
    pub json_mode: bool,
    /// Extra JSON merged into the request body, e.g.
    /// `{"thinking": {"type": "disabled"}}`.
    pub extra_body: String,
    /// Optional additional instructions appended to the system prompt.
    pub custom_instructions: String,
    pub timeout_secs: u64,
}

impl Default for LlmConfig {
    fn default() -> Self {
        LlmConfig {
            base_url: "https://api.deepseek.com".into(),
            api_key: String::new(),
            model: "deepseek-flash".into(),
            target_language: "English".into(),
            temperature: 0.3,
            context_sentences: 4,
            json_mode: true,
            extra_body: String::new(),
            custom_instructions: String::new(),
            timeout_secs: 60,
        }
    }
}

/// One element of the sentence breakdown.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Part {
    pub text: String,
    pub reading: String,
    pub meaning: String,
    /// Grammatical role / form, e.g. "verb, past polite" or "topic particle".
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct GrammarNote {
    pub pattern: String,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Translation {
    pub translation: String,
    #[serde(alias = "literal_translation")]
    pub literal: String,
    pub breakdown: Vec<Part>,
    pub grammar: Vec<GrammarNote>,
    pub notes: String,
    #[serde(skip_deserializing)]
    pub model: String,
    #[serde(skip_deserializing)]
    pub cached: bool,
}

fn system_prompt(cfg: &LlmConfig) -> String {
    let lang = &cfg.target_language;
    let mut s = format!(
        r#"You are an expert Japanese-to-{lang} literary translator and a patient Japanese teacher.
The user is a learner reading a Japanese light novel one sentence at a time.
Translate ONLY the target sentence. Earlier sentences are context only (use them to resolve pronouns, omitted subjects and tone).

Reply with a single JSON object, no markdown fences, with exactly these keys:
{{
  "translation": "natural, faithful {lang} translation of the target sentence",
  "literal": "a more literal {lang} rendering that mirrors the Japanese structure",
  "breakdown": [
    {{"text": "Japanese chunk exactly as it appears in the sentence", "reading": "hiragana reading (empty if kana only)", "meaning": "meaning in {lang}", "role": "part of speech + grammatical function / conjugation, in {lang}"}}
  ],
  "grammar": [
    {{"pattern": "grammar pattern or construction used (e.g. 〜てしまう)", "explanation": "short explanation in {lang} of what it does in this sentence"}}
  ],
  "notes": "optional: nuance, omitted subjects, cultural references, wordplay (in {lang}); empty string if none"
}}
Rules:
- "breakdown" must cover the whole target sentence in order, split into meaningful chunks (words with their attached inflections/particles as separate items when useful). Skip punctuation.
- Keep explanations concise and learner-friendly.
- Write every explanation in {lang}."#
    );
    if !cfg.custom_instructions.trim().is_empty() {
        s.push_str("\n\nAdditional instructions:\n");
        s.push_str(cfg.custom_instructions.trim());
    }
    s
}

fn user_prompt(context: &[String], sentence: &str) -> String {
    let mut s = String::new();
    if !context.is_empty() {
        s.push_str("Context (preceding sentences, do not translate):\n");
        for c in context {
            s.push_str(c);
            s.push('\n');
        }
        s.push('\n');
    }
    s.push_str("Target sentence:\n");
    s.push_str(sentence);
    s
}

fn endpoint(base: &str, path: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    if base.ends_with(path) {
        base.to_string()
    } else {
        format!("{base}{path}")
    }
}

fn client(cfg: &LlmConfig) -> Result<reqwest::Client> {
    crate::http_client()
        .timeout(std::time::Duration::from_secs(cfg.timeout_secs.max(5)))
        .build()
        .map_err(|e| Error::Llm(e.to_string()))
}

fn auth(req: reqwest::RequestBuilder, cfg: &LlmConfig) -> reqwest::RequestBuilder {
    if cfg.api_key.trim().is_empty() {
        req
    } else {
        req.bearer_auth(cfg.api_key.trim())
    }
}

async fn error_text(resp: reqwest::Response) -> Error {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let msg = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(|m| m.as_str()).map(String::from))
        .unwrap_or_else(|| body.chars().take(300).collect());
    Error::Llm(format!("HTTP {status}: {msg}"))
}

/// Raw chat completion; returns the assistant message content.
pub async fn chat(cfg: &LlmConfig, system: &str, user: &str) -> Result<String> {
    if cfg.base_url.trim().is_empty() || cfg.model.trim().is_empty() {
        return Err(Error::Llm("configure the API base URL and model in Settings".into()));
    }
    let mut body = json!({
        "model": cfg.model.trim(),
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ],
        "temperature": cfg.temperature,
        "stream": false
    });
    if cfg.json_mode {
        body["response_format"] = json!({"type": "json_object"});
    }
    if !cfg.extra_body.trim().is_empty() {
        let extra: Value = serde_json::from_str(&cfg.extra_body)
            .map_err(|e| Error::Llm(format!("extra request JSON is invalid: {e}")))?;
        if let (Some(obj), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    let resp = auth(client(cfg)?.post(endpoint(&cfg.base_url, "/chat/completions")), cfg)
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Llm(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(error_text(resp).await);
    }
    let v: Value = resp.json().await.map_err(|e| Error::Llm(e.to_string()))?;
    let content = v
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| Error::Llm("response had no message content".into()))?;
    Ok(strip_think(content))
}

/// Models served by the endpoint (`GET /models`), for the settings screen.
pub async fn list_models(cfg: &LlmConfig) -> Result<Vec<String>> {
    let resp = auth(client(cfg)?.get(endpoint(&cfg.base_url, "/models")), cfg)
        .send()
        .await
        .map_err(|e| Error::Llm(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(error_text(resp).await);
    }
    let v: Value = resp.json().await.map_err(|e| Error::Llm(e.to_string()))?;
    let mut ids: Vec<String> = v
        .get("data")
        .and_then(|d| d.as_array())
        .map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from)).collect())
        .unwrap_or_default();
    ids.sort();
    Ok(ids)
}

/// Removes `<think>…</think>` blocks that some reasoning models emit inline.
fn strip_think(s: &str) -> String {
    let mut out = s.to_string();
    while let (Some(a), Some(b)) = (out.find("<think>"), out.find("</think>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</think>".len(), "");
    }
    out.trim().to_string()
}

/// Parse the model output; tolerate code fences, leading prose, or a plain
/// text answer (which becomes the translation).
pub fn parse_translation(content: &str) -> Translation {
    let trimmed = content.trim();
    let candidate = match (trimmed.find('{'), trimmed.rfind('}')) {
        (Some(a), Some(b)) if b > a => &trimmed[a..=b],
        _ => "",
    };
    if let Ok(t) = serde_json::from_str::<Translation>(candidate) {
        if !t.translation.trim().is_empty() {
            return t;
        }
    }
    Translation { translation: trimmed.trim_matches('`').trim().to_string(), ..Default::default() }
}

impl Core {
    pub fn llm_config(&self) -> Result<LlmConfig> {
        let settings = self.get_settings()?;
        Ok(settings
            .get("llm")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    pub fn cached_translation(&self, book_id: i64, idx: i64, lang: &str) -> Result<Option<Translation>> {
        use rusqlite::OptionalExtension;
        let row: Option<(String, String)> = self
            .db()
            .query_row(
                "SELECT text, model FROM translations WHERE book_id = ?1 AND idx = ?2 AND lang = ?3",
                rusqlite::params![book_id, idx, lang],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.map(|(text, model)| {
            let mut t: Translation =
                serde_json::from_str(&text).unwrap_or_else(|_| Translation { translation: text, ..Default::default() });
            t.model = model;
            t.cached = true;
            t
        }))
    }

    /// Translate sentence `idx` of `book_id`, using the cache unless `force`.
    pub async fn translate(&self, book_id: i64, idx: i64, force: bool) -> Result<Translation> {
        let cfg = self.llm_config()?;
        let lang = cfg.target_language.trim().to_string();
        if !force {
            if let Some(t) = self.cached_translation(book_id, idx, &lang)? {
                return Ok(t);
            }
        }
        let n = cfg.context_sentences.clamp(0, 20);
        let context = self.sentence_texts(book_id, (idx - n).max(0), idx - (idx - n).max(0))?;
        let sentence = self
            .sentence_texts(book_id, idx, 1)?
            .pop()
            .ok_or_else(|| Error::NotFound(format!("sentence {idx} not found")))?;

        let content = chat(&cfg, &system_prompt(&cfg), &user_prompt(&context, &sentence)).await?;
        let mut t = parse_translation(&content);
        t.model = cfg.model.clone();
        self.db().execute(
            "INSERT INTO translations(book_id, idx, lang, text, model, updated_at) VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(book_id, idx, lang) DO UPDATE SET text = excluded.text, model = excluded.model, updated_at = excluded.updated_at",
            rusqlite::params![book_id, idx, lang, serde_json::to_string(&t)?, cfg.model, crate::now_ms()],
        )?;
        Ok(t)
    }

    /// Sends a tiny request to verify the configuration.
    pub async fn test_llm(&self, cfg: &LlmConfig) -> Result<String> {
        let mut c = cfg.clone();
        c.json_mode = false;
        chat(&c, "Reply with the single word: OK", "ping").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_variants() {
        let j = r#"```json
{"translation":"I am a cat.","literal":"As for me, cat is.","breakdown":[{"text":"吾輩","reading":"わがはい","meaning":"I","role":"pronoun"}],"grammar":[],"notes":""}
```"#;
        let t = parse_translation(j);
        assert_eq!(t.translation, "I am a cat.");
        assert_eq!(t.breakdown[0].reading, "わがはい");
        let t = parse_translation("I am a cat.");
        assert_eq!(t.translation, "I am a cat.");
        assert!(t.breakdown.is_empty());
    }

    #[test]
    fn think_and_endpoint() {
        assert_eq!(strip_think("<think>hmm</think>\n{\"a\":1}"), "{\"a\":1}");
        assert_eq!(endpoint("https://api.deepseek.com/", "/chat/completions"), "https://api.deepseek.com/chat/completions");
        assert_eq!(endpoint("http://x/v1/chat/completions", "/chat/completions"), "http://x/v1/chat/completions");
    }
}
