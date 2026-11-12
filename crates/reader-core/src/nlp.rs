//! Morphological analysis with lindera + embedded IPADIC (ADR-0004).

use std::borrow::Cow;
use std::sync::OnceLock;

use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use serde::Serialize;

use crate::segment::RubySpan;
use crate::text::{align_furigana, has_kanji, is_kana, kata_to_hira, Seg};
use crate::{Error, Result};

/// One word of an analysed sentence, ready to be rendered.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Token {
    pub surface: String,
    /// Char offsets in the sentence.
    pub start: usize,
    pub end: usize,
    /// Rendering pieces, with furigana where kanji are present.
    pub segs: Vec<Seg>,
    /// Dictionary (lemma) form as reported by the tokenizer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Hiragana reading of the surface form.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reading: Option<String>,
    /// Coarse part of speech (IPADIC level 1, e.g. `名詞`).
    pub pos: String,
    /// True for punctuation / symbols / whitespace (not clickable).
    pub punct: bool,
}

pub struct Analyzer {
    seg: Segmenter,
}

static ANALYZER: OnceLock<std::result::Result<Analyzer, String>> = OnceLock::new();

impl Analyzer {
    /// Global, lazily initialised analyzer (loading the embedded dictionary
    /// takes a moment, so the app warms it up on a background thread).
    pub fn global() -> Result<&'static Analyzer> {
        ANALYZER
            .get_or_init(|| {
                let dict = load_dictionary("embedded://ipadic").map_err(|e| e.to_string())?;
                Ok(Analyzer { seg: Segmenter::new(Mode::Normal, dict, None) })
            })
            .as_ref()
            .map_err(|e| Error::Nlp(e.clone()))
    }

    /// Tokenise `text`, attach furigana, and prefer the author's ruby
    /// (`hints`) wherever the book provides one.
    pub fn analyze(&self, text: &str, hints: &[RubySpan]) -> Result<Vec<Token>> {
        let mut raw = self.seg.segment(Cow::Borrowed(text)).map_err(|e| Error::Nlp(e.to_string()))?;

        // byte → char offset table
        let mut b2c = vec![0usize; text.len() + 1];
        let mut ci = 0;
        for (bi, ch) in text.char_indices() {
            for k in 0..ch.len_utf8() {
                b2c[bi + k] = ci;
            }
            ci += 1;
        }
        b2c[text.len()] = ci;

        let mut tokens: Vec<Token> = Vec::with_capacity(raw.len());
        let mut last_byte = 0usize;
        for t in raw.iter_mut() {
            if t.byte_start > last_byte {
                // whitespace skipped by the segmenter
                let gap = &text[last_byte..t.byte_start];
                tokens.push(plain_token(gap, b2c[last_byte], b2c[t.byte_start]));
            }
            last_byte = t.byte_end;
            let surface = t.surface.to_string();
            let details: Vec<String> = t.details().iter().map(|s| s.to_string()).collect();
            let pos = details.first().cloned().unwrap_or_default();
            let field = |i: usize| details.get(i).filter(|s| s.as_str() != "*" && !s.is_empty()).cloned();
            let base = field(6);
            let reading = field(7).map(|r| kata_to_hira(&r));
            let punct = pos == "記号" || surface.chars().all(|c| !c.is_alphanumeric() && !is_kana(c));
            let segs = match &reading {
                Some(r) if has_kanji(&surface) => align_furigana(&surface, r),
                _ => vec![Seg::plain(surface.clone())],
            };
            tokens.push(Token {
                start: b2c[t.byte_start],
                end: b2c[t.byte_end],
                surface,
                segs,
                base,
                reading,
                pos,
                punct,
            });
        }
        if last_byte < text.len() {
            tokens.push(plain_token(&text[last_byte..], b2c[last_byte], b2c[text.len()]));
        }

        if !hints.is_empty() {
            tokens = apply_hints(tokens, text, hints);
        }
        Ok(tokens)
    }
}

fn plain_token(s: &str, start: usize, end: usize) -> Token {
    Token {
        surface: s.to_string(),
        start,
        end,
        segs: vec![Seg::plain(s)],
        base: None,
        reading: None,
        pos: "記号".into(),
        punct: true,
    }
}

/// Merge tokens that an author ruby span straddles, then rebuild the segs of
/// every token touched by a span using the author's reading.
fn apply_hints(tokens: Vec<Token>, text: &str, hints: &[RubySpan]) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut merged: Vec<Token> = Vec::with_capacity(tokens.len());
    for t in tokens {
        let straddles = merged.last().is_some_and(|prev: &Token| {
            hints.iter().any(|h| h.start < prev.end && h.end > prev.end && t.start < h.end)
        });
        if straddles {
            let prev = merged.last_mut().unwrap();
            prev.surface.push_str(&t.surface);
            prev.end = t.end;
            prev.reading = match (prev.reading.take(), t.reading) {
                (Some(a), Some(b)) => Some(a + &b),
                _ => None,
            };
            prev.punct = false;
        } else {
            merged.push(t);
        }
    }
    for t in merged.iter_mut() {
        let inside: Vec<&RubySpan> = hints.iter().filter(|h| h.start >= t.start && h.end <= t.end).collect();
        if inside.is_empty() {
            continue;
        }
        let mut segs = Vec::new();
        let mut pos = t.start;
        let mut sorted = inside.clone();
        sorted.sort_by_key(|h| h.start);
        for h in sorted {
            if h.start < pos {
                continue;
            }
            if h.start > pos {
                let s: String = chars[pos..h.start].iter().collect();
                segs.push(Seg::plain(s));
            }
            let s: String = chars[h.start..h.end].iter().collect();
            segs.push(Seg::ruby(s, h.reading.clone()));
            pos = h.end;
        }
        if pos < t.end {
            let s: String = chars[pos..t.end].iter().collect();
            segs.push(Seg::plain(s));
        }
        // Merge adjacent plain segs for tidier output.
        let mut tidy: Vec<Seg> = Vec::new();
        for s in segs {
            match (tidy.last_mut(), &s.ruby) {
                (Some(last), None) if last.ruby.is_none() => last.text.push_str(&s.text),
                _ => tidy.push(s),
            }
        }
        t.segs = tidy;
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_basic() {
        let a = Analyzer::global().unwrap();
        let toks = a.analyze("私は寿司を食べました。", &[]).unwrap();
        let surfaces: Vec<&str> = toks.iter().map(|t| t.surface.as_str()).collect();
        assert_eq!(surfaces.concat(), "私は寿司を食べました。");
        let tabe = toks.iter().find(|t| t.surface == "食べ").unwrap();
        assert_eq!(tabe.base.as_deref(), Some("食べる"));
        assert_eq!(tabe.segs, vec![Seg::ruby("食", "た"), Seg::plain("べ")]);
        assert!(toks.last().unwrap().punct);
        // offsets are contiguous
        for w in toks.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
    }

    #[test]
    fn whitespace_gap_preserved() {
        let a = Analyzer::global().unwrap();
        let text = "なんなの？　その勝算";
        let toks = a.analyze(text, &[]).unwrap();
        assert_eq!(toks.iter().map(|t| t.surface.as_str()).collect::<String>(), text);
    }

    #[test]
    fn author_ruby_wins() {
        let a = Analyzer::global().unwrap();
        let text = "私、小柳奈ノ花は";
        let hints: Vec<RubySpan> = [(2, 3, "こ"), (3, 4, "やなぎ"), (4, 5, "な"), (5, 6, "の"), (6, 7, "か")]
            .iter()
            .map(|(s, e, r)| RubySpan { start: *s, end: *e, reading: r.to_string() })
            .collect();
        let toks = a.analyze(text, &hints).unwrap();
        let rubies: Vec<String> =
            toks.iter().flat_map(|t| t.segs.iter()).filter_map(|s| s.ruby.clone()).collect();
        assert_eq!(rubies, vec!["わたし", "こ", "やなぎ", "な", "の", "か"]);
    }
}
